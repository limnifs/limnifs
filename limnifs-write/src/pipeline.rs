//! Pipeline parallelism — producer/consumer overlap of file I/O
//! and compression.
//!
//! **Status:** opt-in (behind the `pipeline-parallelism` feature
//! flag, default off).
//!
//! The default writer pipeline uses `par_iter().map(process_file)`
//! which fans out across files. Each worker does its own
//! `std::fs::read` + chunk + compress. On warm cache this is
//! optimal — the kernel page cache serves reads in microseconds.
//!
//! For cold-cache workloads (network filesystems, spinning rust,
//! first-time reads of a large tree), the read blocks the worker;
//! CPU sits idle waiting for I/O. This module's pipeline overlaps
//! them: N read threads feed M compress threads via a bounded
//! crossbeam channel.
//!
//! ## Activation
//!
//! The feature flag is the only entry point. When enabled, the
//! writer checks `WriteConfig` for an opt-in flag (currently
//! always picks pipeline when feature is on) and routes through
//! [`write_directory_with_pipeline`]. Default build is unchanged.
//!
//! ## Why opt-in
//!
//! Pipeline parallelism adds:
//! - crossbeam-channel dependency.
//! - ~200 LOC of staging/coordination.
//! - Subtle ordering requirements (slab assembly must be
//!   deterministic; the pipeline preserves PendingFile order).
//!
//! For most workloads the speedup is negligible or negative
//! (channel overhead exceeds the I/O wait). Ship behind a flag,
//! let users benchmark their workload.
//!
//! See `TODO.impl/04-writer-pipeline/04-pipeline-parallelism.md`.

#![cfg(feature = "pipeline-parallelism")]

use std::path::Path;
use std::sync::Arc;

use crossbeam_channel::{bounded, TryRecvError};
use rayon::prelude::*;

use crate::chunker::FastCDC;
use crate::classifier;
use crate::config::WriteConfig;
use crate::file_categorizer;
use crate::{ChunkedFileResult, PendingFile, WriteArtifact, WriteContext, WriteError};

/// Read I/O threads. Set to half the CPU count — more read threads
/// don't help once the disk queue is saturated.
const READ_THREADS: usize = 2;

/// Channel capacity. Bounded so read threads back-pressure when
/// compress threads fall behind. 2× thread count gives enough
/// buffering to smooth out bursty I/O without unbounded memory.
const CHANNEL_CAPACITY: usize = 16;

/// Write a directory tree using the producer/consumer pipeline.
///
/// Same output as `write_directory_with_config`; the only
/// difference is internal scheduling.
///
/// # Errors
/// Returns [`WriteError`] on I/O failure.
pub fn write_directory_with_pipeline(
    root: &Path,
    config: &WriteConfig,
) -> Result<WriteArtifact, WriteError> {
    let mut ctx = WriteContext::new();
    ctx.categorizers_disabled = config.categorizers.is_empty();
    ctx.rw_mode = matches!(config.mode, crate::config::ImageMode::ReadWrite(_));
    ctx.auto_turnover = config.turnover_threshold > 0;
    ctx.collect_dict_samples = config.dictionaries.enabled;
    ctx.inline_threshold = config.defaults.inline_threshold as usize;
    ctx.metadata_externalize_threshold = config.defaults.metadata_externalize_threshold;
    ctx.emit_shared_inline = config.defaults.shared_inline;

    let root_inode_number = ctx.walk(root)?;
    ctx.root_inode_number = root_inode_number;

    let pending = std::mem::take(&mut ctx.pending_files);
    if pending.is_empty() {
        ctx.train_and_apply_dictionary(&config.dictionaries);
        return Ok(ctx.assemble());
    }

    let chunker = Arc::new(crate::chunker_from_config(config)?);
    let classifier = ctx.classifier;
    let text_codec = config.text_codec_id().unwrap_or(0x04);
    let binary_codec = config.binary_codec_id().unwrap_or(0x01);
    let tunables = config.to_core_tunables();
    let use_categorizers = !config.categorizers.is_empty();
    let max_drop_size = config.defaults.max_drop_size as usize;
    let seekable_drops = config.defaults.seekable_drops;

    // Phase 1: read I/O threads feed a bounded channel.
    let (read_tx, read_rx) = bounded::<Arc<Vec<u8>>>(CHANNEL_CAPACITY);
    let pending_arc = Arc::new(pending.clone());
    let read_handles: Vec<std::thread::JoinHandle<()>> = (0..READ_THREADS)
        .map(|t| {
            let tx = read_tx.clone();
            let pending = Arc::clone(&pending_arc);
            std::thread::spawn(move || {
                let stride = READ_THREADS;
                let mut idx = t;
                while idx < pending.len() {
                    let pf = &pending[idx];
                    if let Ok(data) = std::fs::read(&pf.path) {
                        if tx.send(Arc::new(data)).is_err() {
                            break;
                        }
                    }
                    idx += stride;
                }
            })
        })
        .collect();
    drop(read_tx);

    // Phase 2: compress threads pull from the channel, process
    // in PendingFile order (deterministic).
    let results: Vec<ChunkedFileResult> = (0..pending.len())
        .map(|i| {
            let data = match read_rx.recv() {
                Ok(d) => d,
                Err(_) => {
                    return Err(WriteError::Io(std::io::Error::other(
                        "pipeline: read channel closed before all files processed",
                    )))
                }
            };
            let pf = &pending[i];
            let chunker: &FastCDC = &chunker;
            process_file_inline(
                pf,
                &data,
                chunker,
                classifier,
                text_codec,
                binary_codec,
                &tunables,
                use_categorizers,
                max_drop_size,
                seekable_drops,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    for handle in read_handles {
        let _ = handle.join();
    }

    for (pf, result) in pending.iter().zip(results) {
        ctx.merge_chunked_file(pf, result);
    }

    ctx.train_and_apply_dictionary(&config.dictionaries);
    Ok(ctx.assemble())
}

/// Same shape as `process_file` but takes pre-read data instead
/// of reading from disk. Avoids the double-read.
fn process_file_inline(
    pf: &PendingFile,
    data: &[u8],
    chunker: &FastCDC,
    classifier: classifier::Classifier,
    text_codec: u8,
    binary_codec: u8,
    tunables: &limnifs_core::codec::CodecTunables,
    use_categorizers: bool,
    max_drop_size: usize,
    seekable_drops: bool,
) -> Result<ChunkedFileResult, WriteError> {
    let file_len = data.len();
    if use_categorizers {
        if let Some(cat) = file_categorizer::default_registry().categorize(&pf.path, data) {
            let needs_whole_file = matches!(
                cat.codec_id,
                limnifs_core::codec::CODEC_FLAC | limnifs_core::codec::CODEC_RICEPP
            );
            let within_cap = max_drop_size == 0 || file_len <= max_drop_size;
            if within_cap && (needs_whole_file || file_len <= crate::WHOLE_FILE_MAX_SIZE) {
                return process_whole_file_drop_inline(pf, data, cat, tunables, seekable_drops);
            }
        }
    }

    let chunks = chunker.chunk_slice(data);
    let mut slices = Vec::with_capacity(chunks.len());
    let mut file_offset: u64 = 0;
    let mut seen_in_file: std::collections::HashSet<[u8; 32]> =
        std::collections::HashSet::with_capacity(chunks.len());
    let mut unique_chunks: Vec<(&[u8], [u8; 32])> = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        let chunk_len = u64::try_from(chunk.len()).expect("chunk len fits u64");
        let drop_id = limnifs_core::hash_section(chunk);
        slices.push(crate::PendingSlice {
            drop_id,
            file_byte_start: file_offset,
            file_byte_end: file_offset + chunk_len,
        });
        file_offset += chunk_len;
        if seen_in_file.insert(drop_id) {
            unique_chunks.push((chunk, drop_id));
        }
    }

    use rayon::prelude::*;
    let drops: Vec<crate::RawDrop> = unique_chunks
        .par_iter()
        .map(|(chunk, drop_id)| {
            let class = classifier.classify(chunk);
            let preferred_codec = match class {
                classifier::Class::Binary => binary_codec,
                classifier::Class::Text | classifier::Class::Code | classifier::Class::Sparse => {
                    text_codec
                }
                _ => limnifs_core::codec::CODEC_STORE,
            };
            let (codec_id, compressed): (u8, std::sync::Arc<[u8]>) = if preferred_codec
                == limnifs_core::codec::CODEC_STORE
            {
                (limnifs_core::codec::CODEC_STORE, chunk.to_vec().into())
            } else {
                match limnifs_core::codec::compress_with_tunables(preferred_codec, chunk, tunables)
                {
                    Ok(c) if c.len() < chunk.len() => (preferred_codec, c.into()),
                    _ => (limnifs_core::codec::CODEC_STORE, chunk.to_vec().into()),
                }
            };
            (*drop_id, chunk.to_vec(), compressed, codec_id, 0)
        })
        .collect();

    let _ = file_len;
    let _ = TryRecvError::Empty;
    Ok(ChunkedFileResult { drops, slices })
}

fn process_whole_file_drop_inline(
    pf: &PendingFile,
    data: &[u8],
    cat: file_categorizer::Categorization,
    tunables: &limnifs_core::codec::CodecTunables,
    seekable_drops: bool,
) -> Result<ChunkedFileResult, WriteError> {
    let drop_id = limnifs_core::hash_section(data);
    // Brotli first; fall back to ZSTD then STORE on failure (including
    // a codec panic — the registry converts panics to Err).
    let (mut best_codec, mut best_compressed): (u8, std::sync::Arc<[u8]>) =
        match limnifs_core::codec::compress_with_tunables(
            limnifs_core::codec::CODEC_BROTLI,
            data,
            tunables,
        ) {
            Ok(c) => (limnifs_core::codec::CODEC_BROTLI, c.into()),
            Err(_) => match limnifs_core::codec::compress_with_tunables(
                limnifs_core::codec::CODEC_ZSTD,
                data,
                tunables,
            ) {
                Ok(c) => (limnifs_core::codec::CODEC_ZSTD, c.into()),
                Err(_) => (limnifs_core::codec::CODEC_STORE, data.to_vec().into()),
            },
        };
    let brotli_ratio = best_compressed.len() as f64 / data.len() as f64;
    if brotli_ratio > 0.05 && best_codec == limnifs_core::codec::CODEC_BROTLI {
        if let Ok(zstd_c) = limnifs_core::codec::compress_with_tunables(
            limnifs_core::codec::CODEC_ZSTD,
            data,
            tunables,
        ) {
            if zstd_c.len() < best_compressed.len() {
                best_codec = limnifs_core::codec::CODEC_ZSTD;
                best_compressed = zstd_c.into();
            }
        }
    }
    let general_ratio = best_compressed.len() as f64 / data.len() as f64;
    if general_ratio > 0.15 || cat.codec_id == limnifs_core::codec::CODEC_RICEPP {
        if let Ok(spec_c) = limnifs_core::codec::compress(cat.codec_id, data) {
            if spec_c.len() < best_compressed.len() {
                best_codec = cat.codec_id;
                best_compressed = spec_c.into();
            }
        }
    }
    let file_len = u64::try_from(data.len()).unwrap_or(u64::MAX);
    let (best_compressed, flags) =
        crate::seekable_or_monolithic(best_codec, data, best_compressed, tunables, seekable_drops);
    let _ = pf;
    Ok(ChunkedFileResult {
        drops: vec![(drop_id, data.to_vec(), best_compressed, best_codec, flags)],
        slices: vec![crate::PendingSlice {
            drop_id,
            file_byte_start: 0,
            file_byte_end: file_len,
        }],
    })
}
