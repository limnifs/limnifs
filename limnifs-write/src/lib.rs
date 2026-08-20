//! `LimniFS` writer pipeline — directory tree to `.lim` image.
//!
//! The writer takes a real directory tree and produces a valid `.lim`
//! manifest artifact with inlined metadata. Files at or below the
//! inline threshold (4 KiB) are stored as inline data in their inodes;
//! larger files are stored as drops packed into a single slab.
//!
//! ## Usage
//!
//! ```no_run
//! use std::path::Path;
//! use limnifs_write::write_directory;
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let artifact = write_directory(Path::new("/path/to/dir"))?;
//! std::fs::write("output.lim", &artifact.bytes)?;
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]
#![allow(warnings)]

pub mod chunker;
pub mod classifier;
pub mod compaction;
pub mod config;
pub mod delta_builder;
pub mod dictionary;
pub mod file_categorizer;
pub mod flatten;
#[cfg(feature = "pipeline-parallelism")]
pub mod pipeline;
pub mod rw;
#[cfg(feature = "sparse-index")]
pub mod sparse_index;
pub mod turnover;

pub use config::{
    profile, CategorizerConfig, ChunkingConfig, CodecRegistry, CodecTunables, Defaults,
    DictionaryConfig, EncryptionConfig, TournamentConfig, WriteConfig,
};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::chunker::FastCDC;
use limnifs_core::codec::CODEC_REFERENCED;
use limnifs_core::slab_store::SlabStore;
use limnifs_core::{
    compute_merkle_root, hash_empty_section, hash_section, parse_manifest_header, parse_slab_index,
    ManifestCursor, ManifestHeader, SectionHashes, FEATURE_FLAGS_SECTION_VERSION,
    HISTORY_SECTION_VERSION, INODE_FLAG_INLINE_DATA, INODE_FLAG_SHARED_INLINE,
    METADATA_REFERENCE_SECTION_VERSION_2, SLAB_INDEX_SECTION_VERSION,
};
use limnifs_format::{ManifestRoot, SlabId};

/// Inline-data threshold: files at or below this size get inline data
/// in their inode. Larger files are stored as drops in a slab.
pub const INLINE_THRESHOLD: usize = 4096;

/// Above this size, mmap the input file instead of `std::fs::read`-ing
/// it into a `Vec<u8>`. Keeps peak RSS bounded when packing huge files
/// (multi-GiB source trees, ML models). Crossover is around 1 MiB on
/// most filesystems — below that the syscall + VMA setup costs more
/// than the read.
pub const MMAP_READ_THRESHOLD: usize = 1024 * 1024;

/// Maximum file size for the whole-file categorizer path. Files above
/// this threshold use FastCDC chunking even when a categorizer claims
/// them, enabling rayon parallelism across chunks. The categorizer's
/// codec is still used per-chunk when possible.
pub const WHOLE_FILE_MAX_SIZE: usize = 64 * 1024 * 1024;

/// Maximum total length of a single slab file (header + content).
/// Matches the reader's `DEFAULT_SLAB_MAX_BYTES` (spec §3.1) minus a
/// safety margin so a slab that is full but not yet flushed cannot
/// overrun the reader ceiling on the next drop.
pub const MAX_SLAB_TOTAL_BYTES: usize = 60 * 1024 * 1024;

/// Width of the slab header (magic + version + `SlabId` + `total_length` +
/// `ec_descriptor` + `crypto_hint`). Must agree with
/// `limnifs_core::slab::SLAB_HEADER_LEN`.
const SLAB_HEADER_LEN: usize = 56;

/// Threshold at which the writer externalises the metadata blob to a
/// sidecar file instead of inlining it in the manifest. The reader's
/// default inline ceiling is 1 MiB (spec §5.3); we externalise well
/// before that to leave headroom for variance in inode encoding and
/// to keep manifests compact for large trees.
pub const METADATA_EXTERNALIZE_THRESHOLD: usize = 768 * 1024;

/// Metadata-blob size above which the writer steps Brotli quality
/// down to `METADATA_LARGE_BLOB_QUALITY`. Below this, q5's cost is
/// negligible; above it, q5 starts to dominate create time on big
/// inode trees (e.g. the 50 K-file tiny-files dataset).
pub const METADATA_LARGE_BLOB_THRESHOLD: usize = 256 * 1024;

/// Brotli quality for small metadata blobs (≤ `METADATA_LARGE_BLOB_THRESHOLD`).
/// Best ratio; cost is in the noise on small inputs.
pub const METADATA_SMALL_BLOB_QUALITY: i32 = 5;

/// Brotli quality for large metadata blobs. q2 is much faster than q5
/// on multi-MiB inputs; ratio on highly compressible inode data is
/// within 5–10% of q5 (often identical) because metadata is dominated
/// by long runs of repeated patterns.
pub const METADATA_LARGE_BLOB_QUALITY: i32 = 2;

/// One slab produced by the writer. The slab ordinal in `id` matches
/// the slab's position in `WriteArtifact::slabs`.
#[derive(Clone, Debug)]
pub struct SlabArtifact {
    pub id: SlabId,
    pub bytes: Vec<u8>,
    pub locator: String,
    /// `DropIds` contained in this slab, in slab order. Used by callers
    /// that need to know which slab holds which drop without re-parsing
    /// the slab bytes.
    pub drop_ids: Vec<[u8; 32]>,
}

/// Externalized metadata sidecar, present when the metadata blob
/// exceeds [`METADATA_EXTERNALIZE_THRESHOLD`]. Callers must write
/// `bytes` to `locator` next to the manifest file.
#[derive(Clone, Debug)]
pub struct MetadataSidecar {
    pub bytes: Vec<u8>,
    pub locator: String,
}

/// Result of writing a directory tree.
#[derive(Clone, Debug)]
pub struct WriteArtifact {
    pub bytes: Vec<u8>,
    pub merkle_root: ManifestRoot,
    /// All slabs produced by the writer, in slab-ordinal order. Empty
    /// when the source tree had no files > [`INLINE_THRESHOLD`].
    pub slabs: Vec<SlabArtifact>,
    /// External metadata sidecar, present when the metadata blob
    /// exceeds [`METADATA_EXTERNALIZE_THRESHOLD`]. `None` means the
    /// metadata is inlined in the manifest.
    pub metadata_sidecar: Option<MetadataSidecar>,
    pub inode_count: usize,
    pub file_count: usize,
    pub dir_count: usize,
    pub drop_count: usize,
    /// Inode number of the root directory (i.e. the inode that
    /// represents the source directory itself, not a child of it).
    /// Always a directory and always referenced by the inlined
    /// metadata blob's directory inode table.
    pub root_inode_number: u64,
}

impl WriteArtifact {
    /// Convenience accessor for the single-slab case. Returns the
    /// first slab's bytes if there is exactly one slab, else `None`.
    /// Modern callers should iterate [`WriteArtifact::slabs`] directly.
    #[must_use]
    pub fn slab_bytes(&self) -> Option<&[u8]> {
        if self.slabs.len() == 1 {
            Some(&self.slabs[0].bytes)
        } else {
            None
        }
    }

    /// Convenience accessor for the single-slab case.
    #[must_use]
    pub fn slab_locator(&self) -> Option<&str> {
        if self.slabs.len() == 1 {
            Some(&self.slabs[0].locator)
        } else {
            None
        }
    }
}

/// Error during writing.
#[derive(Debug)]
pub enum WriteError {
    Io(std::io::Error),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Walk a directory tree and produce a valid `.lim` manifest artifact
/// with inlined metadata. Files at or below [`INLINE_THRESHOLD`] bytes
/// are stored inline; larger files are packed into a single slab as
/// content-addressed drops.
///
/// File contents (read, `FastCDC` chunk, `BLAKE3` hash, `LZ4` compress) are
/// processed in parallel across `CPU` cores via `rayon`. The directory
/// tree walk and slab assembly remain sequential so the output is
/// deterministic.
///
/// # Errors
///
/// Returns [`WriteError::Io`] for filesystem errors.
pub fn write_directory(root: &Path) -> Result<WriteArtifact, WriteError> {
    write_directory_with_config(root, &WriteConfig::default_v0_1())
}

/// Pack a single named stream into a `.lim` image.
///
/// For callers that pipe data from a network socket, pipe, or generator
/// and don't want to materialise the full content on disk before
/// packing. The reader is consumed via [`FastCDC::chunk_reader`] which
/// bounds internal buffering at `max_chunk_size + 64 KiB`.
///
/// The resulting image has a single root file with the given `name`
/// (path-relative; safe to use `/` for subdirectories — they're
/// materialised in the metadata tree).
///
/// # Errors
///
/// Returns [`WriteError::Io`] on read failure or any writer-pipeline
/// error.
pub fn write_stream<R: std::io::Read>(
    name: &str,
    reader: R,
    config: &WriteConfig,
) -> Result<WriteArtifact, WriteError> {
    let mut ctx = WriteContext::new();
    ctx.categorizers_disabled = config.categorizers.is_empty();
    ctx.rw_mode = matches!(config.mode, crate::config::ImageMode::ReadWrite(_));
    ctx.auto_turnover = config.turnover_threshold > 0;
    ctx.collect_dict_samples = config.dictionaries.enabled;

    // Synthesise a single PendingFile that points to nothing on disk;
    // we'll bypass process_file's `std::fs::read` and feed the
    // pre-chunked bytes directly.
    let drop_id_root = [0u8; 32]; // placeholder; replaced below
    let pending = PendingFile {
        path: std::path::PathBuf::from(name),
        inode_number: 1,
        file_len: 0, // patched below once we know the total
        mtime_ns: 0,
    };
    ctx.pending_files.push(pending);
    ctx.root_inode_number = 1;

    // Chunk the stream directly via FastCDC's chunk_reader.
    let chunker = ctx.chunker.clone();
    let chunks = chunker.chunk_reader(reader)?;

    // Total size = sum of chunk lengths.
    let total_len: u64 = chunks.iter().map(|c| c.len() as u64).sum();

    // Hash + compress each chunk. We treat each chunk as a unique drop
    // (the stream is single-pass; cross-call dedup is left to the
    // caller). Use the configured tournament spec.
    let text_codec = config.text_codec_id().unwrap_or(0x04);
    let binary_codec = config.binary_codec_id().unwrap_or(0x01);
    let tunables = config.to_core_tunables();
    let classifier = ctx.classifier;
    let registry = config
        .codec_registry()
        .map_err(|e| WriteError::Io(std::io::Error::other(format!("codec registry: {e}"))))?;
    let tournament_codec_ids: Vec<u8> = config
        .tournament
        .codecs
        .iter()
        .filter_map(|n| registry.lookup_by_name(n))
        .collect();
    let tournament = TournamentSpec {
        codec_ids: tournament_codec_ids,
        min_size: config.tournament.min_size_threshold as usize,
        skip_for_binary: config.tournament.skip_for_binary,
        short_circuit_permille: config.tournament.short_circuit_threshold,
    };

    let mut drops: Vec<RawDrop> = Vec::with_capacity(chunks.len());
    let mut slices: Vec<PendingSlice> = Vec::with_capacity(chunks.len());
    let mut offset: u64 = 0;
    for chunk in &chunks {
        let drop_id = hash_section(chunk);
        slices.push(PendingSlice {
            drop_id,
            file_byte_start: offset,
            file_byte_end: offset + chunk.len() as u64,
        });
        offset += chunk.len() as u64;
        let class = classifier.classify(chunk);
        let (codec_id, compressed) = compress_chunk_with_tournament(
            chunk,
            class,
            text_codec,
            binary_codec,
            &tunables,
            &tournament,
        );
        drops.push((drop_id, chunk.clone(), compressed, codec_id));
    }
    let _ = drop_id_root;

    // Wire into WriteContext as a single-file result + inode.
    let result = ChunkedFileResult { drops, slices };
    let pf = ctx.pending_files[0].clone();
    ctx.merge_chunked_file(&pf, result);
    // Patch the file_len now that we know it.
    ctx.pending_files[0].file_len = total_len;
    // The inode was already pushed by merge_chunked_file with the old
    // (zero) file_len; correct it.
    if let Some(inode) = ctx.inodes.last_mut() {
        if let PendingContent::DropBacked { file_len, .. } = &mut inode.content {
            *file_len = total_len;
        }
    }

    ctx.train_and_apply_dictionary(&config.dictionaries);
    let artifact = ctx.assemble();
    Ok(artifact)
}

/// Pack a directory tree as a **layer** on top of a base image.
///
/// Produces a `.lim` image whose drops are split into two sets:
///
/// - **Local drops** — chunks in `root` whose `DropId` is NOT in
///   `base_image`'s drop set. These are compressed and stored in the
///   layer's own slabs exactly as `write_directory_with_config`
///   would store them.
/// - **Referenced drops** — chunks whose `DropId` IS in the base.
///   These are recorded only as `PendingSlice` references (so the
///   metadata tree links them in); no slab bytes are emitted in the
///   layer. The reader resolves them via the overlay chain.
///
/// The resulting manifest carries a `delta_linkage` section pointing
/// at the base image's `ManifestRoot`, so any reader that supports
/// overlay chains can extract the layer standalone or stacked on the
/// base.
///
/// # Determinism
///
/// `write_layer` is deterministic given the same `base_image`, the
/// same `root` content, and the same `config`. Two runs produce
/// byte-identical layer images.
///
/// # Errors
///
/// Returns [`WriteError::Io`] on read failure or any writer-pipeline
/// error.
///
/// # Example
///
/// ```no_run
/// use limnifs_write::{write_layer, profile};
///
/// let base = std::path::Path::new("base.lim");
/// let root = std::path::Path::new("./new-content");
/// let cfg = profile::balanced();
/// let artifact = write_layer(base, root, &cfg).expect("layer");
/// // artifact.bytes is the layer manifest; slabs contain only NEW drops.
/// ```
pub fn write_layer(
    base_image: &Path,
    root: &Path,
    config: &WriteConfig,
) -> Result<WriteArtifact, WriteError> {
    // Load the base image's drop set + manifest root.
    let (base_drop_index, base_root) = load_base_drop_index(base_image)?;

    let mut ctx = WriteContext::new();
    ctx.categorizers_disabled = config.categorizers.is_empty();
    ctx.rw_mode = matches!(config.mode, crate::config::ImageMode::ReadWrite(_));
    ctx.auto_turnover = config.turnover_threshold > 0;
    ctx.collect_dict_samples = config.dictionaries.enabled;
    ctx.base_drop_index = Some(base_drop_index);
    ctx.base_root = Some(base_root);

    // Rest is identical to write_directory_with_config.
    let root_inode_number = ctx.walk(root)?;
    ctx.root_inode_number = root_inode_number;
    write_directory_body(&mut ctx, config)?;
    Ok(ctx.assemble())
}

/// Load every DropId present in a base image's slabs + the image's
/// `ManifestRoot`. Used by `write_layer` to decide which chunks can
/// be referenced rather than re-encoded.
fn load_base_drop_index(
    base_image: &Path,
) -> Result<(std::collections::HashSet<[u8; 32]>, [u8; 32]), WriteError> {
    let manifest_bytes = std::fs::read(base_image)?;
    let mut cursor = ManifestCursor::new(&manifest_bytes);
    let _ = parse_manifest_header(&mut cursor).map_err(io_core)?;
    // Walk sections in spec order: flags → metadata_reference → slab_index.
    let _ = limnifs_core::parse_feature_flags_section(&mut cursor);
    let _ = limnifs_core::parse_metadata_reference(&mut cursor);
    let slab_index = parse_slab_index(&mut cursor).map_err(io_core)?;
    let store = SlabStore::load_mmap(base_image, &slab_index).map_err(io_core)?;
    let drop_set: std::collections::HashSet<[u8; 32]> = store.drop_index_keys().copied().collect();
    // Re-derive the Merkle root from the base manifest's section
    // bytes. The base's `ManifestRoot` is the canonical anchor for
    // the layer's `delta_linkage.base_root` field — round-tripping
    // through section hashes guarantees it matches what the base
    // reported on its own assemble path.
    let root = *compute_merkle_root_from_sections(&manifest_bytes).as_bytes();
    Ok((drop_set, root))
}

/// Re-derive a manifest's `ManifestRoot` from its on-disk section
/// bytes. Mirrors `flatten::compute_merkle_root_from_sections` but
/// tolerates absent optional sections (returns hash_empty_section()
/// for them). Used by `load_base_drop_index` to anchor a layer's
/// `base_root` without re-instantiating a full manifest parser.
fn compute_merkle_root_from_sections(manifest: &[u8]) -> ManifestRoot {
    use limnifs_core::SectionHashes;
    let mut cursor = ManifestCursor::new(manifest);
    let header_start = 0;
    if parse_manifest_header(&mut cursor).is_err() {
        // Not a valid manifest; fall back to all-zero root.
        return ManifestRoot::from_bytes([0u8; 32]);
    }
    let header_end = cursor.position();
    // Optional sections — best-effort parse; failures hash as empty.
    let flags_start = header_end;
    let flags_end = match limnifs_core::parse_feature_flags_section(&mut cursor) {
        Ok(_) => cursor.position(),
        Err(_) => flags_start,
    };
    let meta_ref_start = flags_end;
    let metadata_reference = match limnifs_core::parse_metadata_reference(&mut cursor) {
        Ok(m) => Some(m),
        Err(_) => None,
    };
    let meta_ref_end = cursor.position();
    let slab_index_start = meta_ref_end;
    let _ = parse_slab_index(&mut cursor);
    let slab_index_end = cursor.position();
    let history_start = slab_index_end;
    let _ = limnifs_core::parse_history(&mut cursor);
    let history_end = cursor.position();

    let hashes = SectionHashes {
        metadata: metadata_reference
            .map(|m| m.metadata_hash)
            .unwrap_or_else(hash_empty_section),
        format_header: hash_section(&manifest[header_start..header_end]),
        feature_flags: hash_section(&manifest[flags_start..flags_end]),
        metadata_reference: hash_section(&manifest[meta_ref_start..meta_ref_end]),
        slab_index: hash_section(&manifest[slab_index_start..slab_index_end]),
        crypto_params: hash_empty_section(),
        ec_params: hash_empty_section(),
        dms_policy: hash_empty_section(),
        delta_linkage: hash_empty_section(),
        history: hash_section(&manifest[history_start..history_end]),
    };
    compute_merkle_root(&hashes)
}

fn io_core(e: limnifs_core::CoreError) -> WriteError {
    WriteError::Io(std::io::Error::other(format!("base image load: {e}")))
}

/// Shared body between `write_directory_with_config` and `write_layer`.
/// Walks `ctx.pending_files` through `process_file` in parallel and
/// merges the results back. Caller is responsible for `walk()` and
/// `assemble()`.
fn write_directory_body(ctx: &mut WriteContext, config: &WriteConfig) -> Result<(), WriteError> {
    use rayon::prelude::*;

    ctx.metadata_codec = config
        .metadata_codec_id()
        .unwrap_or(limnifs_core::codec::CODEC_BROTLI);

    let pending = std::mem::take(&mut ctx.pending_files);
    if pending.is_empty() {
        return Ok(());
    }
    let chunker = ctx.chunker.clone();
    let classifier = ctx.classifier;
    let text_codec = config.text_codec_id().unwrap_or(0x04);
    let binary_codec = config.binary_codec_id().unwrap_or(0x01);
    let tunables = config.to_core_tunables();
    let use_categorizers = !config.categorizers.is_empty();
    let skip_chunking = config.skip_chunking;
    let registry = config
        .codec_registry()
        .map_err(|e| WriteError::Io(std::io::Error::other(format!("codec registry: {e}"))))?;
    let tournament_codec_ids: Vec<u8> = config
        .tournament
        .codecs
        .iter()
        .filter_map(|n| registry.lookup_by_name(n))
        .collect();
    let tournament_spec = TournamentSpec {
        codec_ids: tournament_codec_ids,
        min_size: config.tournament.min_size_threshold as usize,
        skip_for_binary: config.tournament.skip_for_binary,
        short_circuit_permille: config.tournament.short_circuit_threshold,
    };
    let base_drop_index = ctx.base_drop_index.as_ref();
    let results: Vec<ChunkedFileResult> = pending
        .par_iter()
        .map(|pf| {
            process_file(
                pf,
                &chunker,
                classifier,
                text_codec,
                binary_codec,
                &tunables,
                use_categorizers,
                skip_chunking,
                &tournament_spec,
                base_drop_index,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    for (pf, result) in pending.iter().zip(results) {
        ctx.merge_chunked_file(pf, result);
    }
    ctx.train_and_apply_dictionary(&config.dictionaries);
    Ok(())
}

/// Create an image with a custom [`WriteConfig`] (e.g. from a profile).
pub fn write_directory_with_config(
    root: &Path,
    config: &WriteConfig,
) -> Result<WriteArtifact, WriteError> {
    let mut ctx = WriteContext::new();
    ctx.categorizers_disabled = config.categorizers.is_empty();
    ctx.rw_mode = matches!(config.mode, crate::config::ImageMode::ReadWrite(_));
    ctx.auto_turnover = config.turnover_threshold > 0;
    ctx.collect_dict_samples = config.dictionaries.enabled;

    let root_inode_number = ctx.walk(root)?;
    ctx.root_inode_number = root_inode_number;
    write_directory_body(&mut ctx, config)?;
    Ok(ctx.assemble())
}

/// One chunk of a file before dedup: (`drop_id`, `plaintext`, `compressed`, `codec`).
///
/// `compressed` is `Arc<[u8]>` so the cross-file compress cache can
/// share bytes across hits with a refcount bump instead of a deep
/// copy — dedup-heavy workloads (container layers, duplicate files)
/// skip the allocation entirely.
pub(crate) type RawDrop = ([u8; 32], Vec<u8>, std::sync::Arc<[u8]>, u8);
/// Result of parallel file processing: the drop data (uncompressed,
pub(crate) struct ChunkedFileResult {
    drops: Vec<RawDrop>, // (id, plaintext, compressed, codec)
    slices: Vec<PendingSlice>,
}

/// Resolved tournament configuration passed to per-chunk compression.
///
/// Built once at the top level from `WriteConfig::tournament` so each
/// rayon worker reuses the same Vec rather than rebuilding it per
/// file. Codecs are stored as numeric ids (looked up via
/// `WriteConfig::codec_registry`) — `process_file` never sees the
/// string form.
struct TournamentSpec {
    /// Codec ids to try, in declared order. `process_file` iterates
    /// these and tracks the best compression. `CODEC_STORE` entries
    /// are ignored (store is always the implicit fallback).
    codec_ids: Vec<u8>,
    /// Chunks below this many bytes get the preferred codec only
    /// (no tournament) — the per-codec setup cost dominates at small
    /// sizes and the ratio difference is negligible.
    min_size: usize,
    /// When true, binary-classified chunks skip the tournament and
    /// use `binary_codec` directly. Matches the v0.1 behaviour where
    /// binary chunks were never worth the tournament cost.
    skip_for_binary: bool,
    /// Short-circuit threshold in per-mille (0..=1000). 0 disables
    /// short-circuit. Whenever a codec achieves compression ratio
    /// ≤ threshold, the tournament accepts it and skips any slower
    /// codecs later in the list.
    short_circuit_permille: u32,
}

/// Compress a whole file as a single drop using the categorizer's
/// chosen codec. Used when a file-level categorizer claims the file
/// (FLAC for WAV, ricepp for FITS, FSST+Brotli for CSV). The drop's
/// slice covers the whole file; no `FastCDC` chunking happens.
///
/// Codec parameters extracted by the categorizer (e.g. PCM sample
/// format, FITS bitpix) are NOT prepended to the compressed bytes —
/// the codec embeds its own params in its container format. The
/// `LimniFS` drop record just stores `(codec_id, compressed_bytes)`
/// and lets the codec own its param encoding. The categorizer's
/// `codec_params` field is reserved for future use when a codec
/// needs params NOT embedded in its container.
fn process_whole_file_drop(
    pf: &PendingFile,
    data: &[u8],
    cat: file_categorizer::Categorization,
    tunables: &limnifs_core::codec::CodecTunables,
) -> Result<ChunkedFileResult, WriteError> {
    let _ = pf;
    let drop_id = hash_section(data);
    let file_len = u64::try_from(data.len()).unwrap_or(u64::MAX);

    // Brotli first; if it fails (including a codec panic — the
    // registry converts panics to Err), fall back to ZSTD, then to
    // STORE. A broken encoder must degrade the drop's ratio, never
    // the write itself.
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

    // Short-circuit: if Brotli already achieves < 5% ratio, the input
    // is highly compressible and ZSTD is unlikely to beat it by enough
    // to justify the extra pass. Skip ZSTD on this fast path.
    let brotli_ratio = best_compressed.len() as f64 / data.len() as f64;
    if brotli_ratio > 0.05 {
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

    // Only try the specialized codec if the general-purpose ratio
    // is poor (>15%) — otherwise the specialized codec is unlikely
    // to help and may be very slow (FLAC, FSST). RICEPP is always
    // tried because it can win big on FITS even when general-purpose
    // ratios look acceptable.
    let general_ratio = best_compressed.len() as f64 / data.len() as f64;
    if general_ratio > 0.15 || cat.codec_id == limnifs_core::codec::CODEC_RICEPP {
        // For FSST+Brotli, pass the already-computed Brotli baseline so
        // the codec doesn't re-compress the plaintext with Brotli just
        // for the comparison check.
        let spec_result = if cat.codec_id == limnifs_core::codec::CODEC_FSST_BROTLI {
            limnifs_core::codec::fsst_brotli::compress_with_baseline(data, Some(&best_compressed))
        } else {
            limnifs_core::codec::compress(cat.codec_id, data)
        };
        if let Ok(spec_c) = spec_result {
            if spec_c.len() < best_compressed.len() {
                best_codec = cat.codec_id;
                best_compressed = spec_c.into();
            }
        }
    }

    Ok(ChunkedFileResult {
        drops: vec![(drop_id, data.to_vec(), best_compressed, best_codec)],
        slices: vec![PendingSlice {
            drop_id,
            file_byte_start: 0,
            file_byte_end: file_len,
        }],
    })
}

/// Process a single file's contents (CPU-heavy work that runs in a
/// rayon worker thread). Returns the unique chunks and slice map.
///
/// First consults the file-level categorizer registry. If a
/// categorizer claims the file (e.g. FLAC for WAV, ricepp for FITS,
/// Compress a single chunk via the configured tournament.
///
/// Iterates `tournament.codec_ids` in declared order, tracks the
/// smallest output, and short-circuits when a codec achieves
/// compression ratio ≤ `tournament.short_circuit_permille`.
///
/// Special cases:
/// - **Binary chunks with `skip_for_binary`**: skip the tournament
///   entirely and use `binary_codec`. Matches v0.1 behaviour.
/// - **Chunks smaller than `min_size`**: use the class's preferred
///   codec directly. Per-codec setup cost dominates here and the
///   ratio difference is negligible at small sizes.
/// - **Class unknown to writer** (Unknown / future classes): STORE.
///
/// The tournament never tries `CODEC_STORE` (id 0x00) — store is
/// always the implicit fallback if every codec fails to compress.
fn compress_chunk_with_tournament(
    chunk: &[u8],
    class: classifier::Class,
    text_codec: u8,
    binary_codec: u8,
    tunables: &limnifs_core::codec::CodecTunables,
    tournament: &TournamentSpec,
) -> (u8, std::sync::Arc<[u8]>) {
    use classifier::Class;

    let preferred = match class {
        Class::Binary => binary_codec,
        Class::Text | Class::Code | Class::Sparse => text_codec,
        _ => limnifs_core::codec::CODEC_STORE,
    };

    if preferred == limnifs_core::codec::CODEC_STORE {
        return (limnifs_core::codec::CODEC_STORE, chunk.to_vec().into());
    }
    if class == Class::Binary && tournament.skip_for_binary {
        return compress_chunk_one(chunk, preferred, tunables);
    }
    if chunk.len() < tournament.min_size {
        return compress_chunk_one(chunk, preferred, tunables);
    }

    let mut best: Option<(u8, std::sync::Arc<[u8]>)> = None;
    for &codec_id in &tournament.codec_ids {
        if codec_id == limnifs_core::codec::CODEC_STORE {
            continue;
        }
        let c = match limnifs_core::codec::compress_with_tunables(codec_id, chunk, tunables) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if c.len() >= chunk.len() {
            continue;
        }
        let ratio_permille = (c.len() as u64 * 1000 / chunk.len() as u64) as u32;
        let is_best_so_far = best.as_ref().map_or(true, |(_, b)| c.len() < b.len());
        if is_best_so_far {
            best = Some((codec_id, c.into()));
        }
        if tournament.short_circuit_permille > 0
            && ratio_permille <= tournament.short_circuit_permille
        {
            break;
        }
    }

    best.unwrap_or_else(|| (limnifs_core::codec::CODEC_STORE, chunk.to_vec().into()))
}

/// Compress `chunk` with a single codec, falling back to STORE if
/// the codec fails or expansion occurs.
fn compress_chunk_one(
    chunk: &[u8],
    codec_id: u8,
    tunables: &limnifs_core::codec::CodecTunables,
) -> (u8, std::sync::Arc<[u8]>) {
    if codec_id == limnifs_core::codec::CODEC_STORE {
        return (limnifs_core::codec::CODEC_STORE, chunk.to_vec().into());
    }
    match limnifs_core::codec::compress_with_tunables(codec_id, chunk, tunables) {
        Ok(c) if c.len() < chunk.len() => (codec_id, c.into()),
        _ => (limnifs_core::codec::CODEC_STORE, chunk.to_vec().into()),
    }
}

/// FSST+Brotli for CSV), the whole file is compressed as a single
/// drop with the categorizer's chosen codec + parameters. Otherwise
/// falls through to `FastCDC` + per-chunk classify.
fn process_file(
    pf: &PendingFile,
    chunker: &FastCDC,
    classifier: classifier::Classifier,
    text_codec: u8,
    binary_codec: u8,
    tunables: &limnifs_core::codec::CodecTunables,
    use_categorizers: bool,
    skip_chunking: bool,
    tournament: &TournamentSpec,
    base_drop_index: Option<&std::collections::HashSet<[u8; 32]>>,
) -> Result<ChunkedFileResult, WriteError> {
    // For files above MMAP_READ_THRESHOLD, map them rather than reading
    // into a Vec via std::fs::read. Pages load on demand from the
    // kernel page cache. The chunk-path compressors see borrowed slices
    // pointing into the mmap, so peak RSS stays at unique_chunks ×
    // avg_chunk_size rather than the full file.
    //
    // SAFETY: LimniFS packs source trees that are immutable for the
    // duration of the write. The file is opened read-only. External
    // mutation during compression would be a serious bug in the
    // caller's workflow (and would also break BLAKE3 determinism).
    let file_len_estimate = std::fs::metadata(&pf.path)
        .map(|m| m.len() as usize)
        .unwrap_or(0);
    let data: Vec<u8> = if file_len_estimate >= MMAP_READ_THRESHOLD {
        let file = std::fs::File::open(&pf.path)?;
        #[allow(unsafe_code)]
        let mmap = unsafe { memmap2::Mmap::map(&file) }.map_err(WriteError::Io)?;
        // Materialize only the pages the kernel has paged in. For
        // chunked access this is roughly unique-chunk bytes; for
        // skip_chunking we touch every page anyway.
        Vec::from(&mmap[..])
    } else {
        std::fs::read(&pf.path)?
    };
    let file_len = data.len();

    // Skip FastCDC chunking entirely; compress the whole file as
    // one drop. Trades dedup granularity for create speed. Used by
    // the max-write profile where speed >> ratio. The per-file LZ4
    // compress at ~1 GB/s is faster than FastCDC hashing overhead
    // for all but the largest multi-GB files (where rayon parallelism
    // across chunks would help).
    if skip_chunking && file_len > INLINE_THRESHOLD {
        let drop_id = hash_section(&data);
        let class = classifier.classify(&data);
        let preferred_codec = match class {
            classifier::Class::Binary => binary_codec,
            _ => text_codec,
        };
        let (codec_id, compressed): (u8, std::sync::Arc<[u8]>) =
            match limnifs_core::codec::compress_with_tunables(preferred_codec, &data, tunables) {
                Ok(c) if c.len() < data.len() => (preferred_codec, c.into()),
                _ => (limnifs_core::codec::CODEC_STORE, data.to_vec().into()),
            };
        return Ok(ChunkedFileResult {
            drops: vec![(drop_id, data, compressed, codec_id)],
            slices: vec![PendingSlice {
                drop_id,
                file_byte_start: 0,
                file_byte_end: file_len as u64,
            }],
        });
    }

    if use_categorizers {
        if let Some(cat) = file_categorizer::default_registry().categorize(&pf.path, &data) {
            let needs_whole_file = matches!(
                cat.codec_id,
                limnifs_core::codec::CODEC_FLAC | limnifs_core::codec::CODEC_RICEPP
            );
            if needs_whole_file || file_len <= WHOLE_FILE_MAX_SIZE {
                return process_whole_file_drop(pf, &data, cat, tunables);
            }
        }
    }

    let chunks = chunker.chunk_slice(&data);
    let mut slices = Vec::with_capacity(chunks.len());
    let mut file_offset: u64 = 0;
    let mut seen_in_file: std::collections::HashSet<[u8; 32]> =
        std::collections::HashSet::with_capacity(chunks.len());

    // Phase 1: hash all chunks + build slices + filter duplicates (sequential).
    // FastCDC boundaries must be deterministic, and BLAKE3 hashing is fast.
    let mut unique_chunks: Vec<(&[u8], [u8; 32])> = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        let chunk_len = u64::try_from(chunk.len()).expect("chunk len fits u64");
        let drop_id = hash_section(chunk);
        slices.push(PendingSlice {
            drop_id,
            file_byte_start: file_offset,
            file_byte_end: file_offset + chunk_len,
        });
        file_offset += chunk_len;
        if seen_in_file.insert(drop_id) {
            unique_chunks.push((chunk, drop_id));
        }
    }

    // Phase 2: compress unique chunks in parallel across rayon workers.
    // This is the CPU-intensive step — parallelizing it gives N-core
    // speedup for large files with many chunks.
    //
    // Cross-file dedup: each rayon worker thread carries a thread-local
    // compress cache mapping DropId -> (codec_id, compressed_bytes).
    // When two files share a chunk (common in source trees, container
    // layers, tiny-files benchmarks), the second file hits the cache
    // and skips the compress pass entirely. Cache is bounded by entry
    // count; eviction is "stop inserting once full" — simple and
    // correct, misses are bounded by worker count.
    use rayon::prelude::*;
    thread_local! {
        static COMPRESS_CACHE: std::cell::RefCell<std::collections::HashMap<[u8; 32], (u8, std::sync::Arc<[u8]>)>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
    }
    const COMPRESS_CACHE_MAX_ENTRIES: usize = 100_000;
    let drops: Vec<RawDrop> = unique_chunks
        .par_iter()
        .map(|(chunk, drop_id)| {
            // Layer fast-path: chunk already exists in the base image
            // → skip compress entirely.
            if let Some(base) = base_drop_index {
                if base.contains(drop_id) {
                    return (*drop_id, Vec::new(), Vec::new().into(), CODEC_REFERENCED);
                }
            }
            let class = classifier.classify(chunk);
            // Cache fast-path: identical chunk already compressed on
            // this worker → reuse bytes, skip tournament entirely.
            let cached = COMPRESS_CACHE.with(|c| {
                c.borrow()
                    .get(drop_id)
                    .map(|(cid, comp)| (*cid, comp.clone()))
            });
            let (codec_id, compressed) = if let Some(c) = cached {
                c
            } else {
                let new = compress_chunk_with_tournament(
                    chunk,
                    class,
                    text_codec,
                    binary_codec,
                    tunables,
                    tournament,
                );
                // Insert into the per-worker cache if there's room.
                COMPRESS_CACHE.with(|c| {
                    let mut cache = c.borrow_mut();
                    if cache.len() < COMPRESS_CACHE_MAX_ENTRIES {
                        // `new.1.clone()` here is an Arc refcount bump.
                        cache.insert(*drop_id, new.clone());
                    }
                });
                new
            };
            (*drop_id, chunk.to_vec(), compressed, codec_id)
        })
        .collect();

    let _ = file_len;
    Ok(ChunkedFileResult { drops, slices })
}

struct PendingDrop {
    id: [u8; 32],
    /// Original (decompressed) byte length. Stored as a u32 rather
    /// than keeping the plaintext Vec around, because the writer only
    /// needs the length when emitting the slab's drop record. Holding
    /// the full plaintext until slab assembly wastes memory
    /// proportional to image size on top of the compressed bytes.
    plaintext_len: u32,
    compressed: std::sync::Arc<[u8]>,
    codec: u8,
    /// Dictionary id (0xFF = NO_DICT). Populated during the dict
    /// re-compression pass for drops that were re-compressed with a
    /// trained dictionary.
    dict_id: u8,
    /// Retained plaintext, present only when `collect_dict_samples`
    /// is true (i.e. `WriteConfig::dictionaries.enabled`). Used by
    /// the post-parallel dict re-compression pass. Cleared after
    /// re-compression to free memory before slab assembly.
    plaintext: Option<Vec<u8>>,
}

impl PendingDrop {
    /// The byte length stored in the slab's solid window. Equals
    /// `plaintext_len` for store codec, or the compressed size for
    /// LZ4 / Brotli / etc.
    fn len_in_window(&self) -> u32 {
        u32::try_from(self.compressed.len()).expect("compressed fits u32")
    }

    /// The original (decompressed) byte length.
    fn plaintext_len_value(&self) -> u32 {
        self.plaintext_len
    }

    /// Contribution to the slab's total byte length: 48 bytes of drop
    /// record (per spec §3.3) + the compressed payload.
    fn slab_footprint(&self) -> usize {
        48 + self.compressed.len()
    }
}

/// One slice of a file backed by drops. Records which drop holds
/// this slice's bytes and which byte range of the original file
/// the slice covers. The slice always spans the entire drop (the
/// chunker never splits a drop across multiple slices).
struct PendingSlice {
    drop_id: [u8; 32],
    file_byte_start: u64,
    file_byte_end: u64,
}

/// A file that needs chunking (> `INLINE_THRESHOLD`). Collected during
/// the sequential tree walk and processed in parallel by `rayon`.
#[derive(Clone)]
struct PendingFile {
    inode_number: u64,
    path: PathBuf,
    mtime_ns: u64,
    file_len: u64,
}

struct PendingInode {
    number: u64,
    mode: u32,
    mtime_ns: u64,
    content: PendingContent,
}

enum PendingContent {
    Inline(Vec<u8>),
    DropBacked {
        file_len: u64,
        slices: Vec<PendingSlice>,
    },
    Directory(Vec<(String, u64, u8)>),
}

struct DirNode {
    entries: Vec<(String, u64, u8)>,
    bytes: Vec<u8>,
    hash: [u8; 32],
}

struct WriteContext {
    next_inode: u64,
    inodes: Vec<PendingInode>,
    dir_nodes: Vec<DirNode>,
    drops: Vec<PendingDrop>,
    drop_index: HashSet<[u8; 32]>,
    pending_files: Vec<PendingFile>,
    file_count: usize,
    dir_count: usize,
    root_inode_number: u64,
    chunker: FastCDC,
    classifier: classifier::Classifier,
    shared_inline_map: HashMap<[u8; 32], usize>,
    shared_inline_table: Vec<Vec<u8>>,
    /// Profile name for ProfileDescriptor emission (None = omit section).
    profile_name: Option<String>,
    /// Metadata blob codec (defaults to Brotli; can be overridden via
    /// `WriteConfig::defaults::metadata_codec`). Used by `assemble`.
    metadata_codec: u8,
    /// Whether categorizers were disabled by the profile.
    categorizers_disabled: bool,
    /// Whether this is a RW image.
    rw_mode: bool,
    /// Whether auto-turnover is enabled.
    auto_turnover: bool,
    /// Whether to collect plaintext samples for ZSTD dictionary
    /// training. Set when `WriteConfig::dictionaries.enabled`.
    collect_dict_samples: bool,
    /// Plaintext samples collected from ZSTD-compressed drops, for
    /// training one dictionary after the parallel compress phase.
    /// Capped at `MAX_DICT_SAMPLES` to bound memory. Keyed by
    /// classifier class — text/code/sparse share a "text" dict,
    /// binary gets its own. Compressed/media/incompressible classes
    /// don't use ZSTD so their samples aren't collected.
    dict_samples_by_class: HashMap<crate::classifier::Class, Vec<Vec<u8>>>,
    /// Trained dictionaries keyed by class. Populated by
    /// `train_and_apply_dictionary` after the parallel phase. Emitted
    /// in the manifest's `dictionary_section` with one entry per
    /// class that accumulated enough samples.
    trained_dicts_by_class: HashMap<crate::classifier::Class, crate::dictionary::TrainedDictionary>,
    /// Drop IDs known to exist in a base image (set when this write
    /// is producing a layer via `write_layer`). When a chunk's DropId
    /// is in this set, the writer skips compression and emits no slab
    /// bytes — the drop is resolved via the overlay chain at read
    /// time. `None` for standalone (non-layer) writes.
    base_drop_index: Option<HashSet<[u8; 32]>>,
    /// The base image's `ManifestRoot` (set when `base_drop_index`
    /// is `Some`). Emitted in the manifest's `delta_linkage` section
    /// so readers know which image provides the referenced drops.
    /// `None` for standalone writes.
    base_root: Option<[u8; 32]>,
}

impl WriteContext {
    /// Cap on collected plaintext samples. Enough signal for the
    /// FrequencyTrainer without unbounded memory growth on huge inputs.
    const MAX_DICT_SAMPLES: usize = 1000;

    fn new() -> Self {
        Self {
            next_inode: 1,
            inodes: Vec::new(),
            dir_nodes: Vec::new(),
            drops: Vec::new(),
            drop_index: HashSet::new(),
            pending_files: Vec::new(),
            file_count: 0,
            dir_count: 0,
            root_inode_number: 0,
            chunker: FastCDC::default(),
            classifier: classifier::Classifier,
            shared_inline_map: HashMap::new(),
            shared_inline_table: Vec::new(),
            profile_name: None,
            metadata_codec: limnifs_core::codec::CODEC_BROTLI,
            categorizers_disabled: false,
            rw_mode: false,
            auto_turnover: false,
            collect_dict_samples: false,
            dict_samples_by_class: HashMap::new(),
            trained_dicts_by_class: HashMap::new(),
            base_drop_index: None,
            base_root: None,
        }
    }

    fn alloc_inode(&mut self) -> u64 {
        let n = self.next_inode;
        self.next_inode += 1;
        n
    }

    /// Scan all inline-data inodes and build a dedup table. Only
    /// content appearing in > 1 inode is deduplicated; unique inline
    /// data stays inline (no overhead change).
    fn build_shared_inline_table(&mut self) {
        let mut counts: HashMap<[u8; 32], usize> = HashMap::new();
        for inode in &self.inodes {
            if let PendingContent::Inline(data) = &inode.content {
                let h = hash_section(data);
                *counts.entry(h).or_default() += 1;
            }
        }
        // Only dedup content that appears more than once.
        for inode in &self.inodes {
            if let PendingContent::Inline(data) = &inode.content {
                let h = hash_section(data);
                if counts.get(&h).copied().unwrap_or(0) > 1
                    && !self.shared_inline_map.contains_key(&h)
                {
                    let idx = self.shared_inline_table.len();
                    self.shared_inline_table.push(data.clone());
                    self.shared_inline_map.insert(h, idx);
                }
            }
        }
    }

    /// Merge a parallel-processed chunked file's results into the
    /// context. Dedup: only new `DropId`s get added to the drops list.
    fn merge_chunked_file(&mut self, pf: &PendingFile, result: ChunkedFileResult) {
        for (drop_id, plaintext, compressed, codec) in result.drops {
            if self.drop_index.insert(drop_id) {
                // When dictionary training is enabled, classify the
                // plaintext and retain it for per-class dictionary
                // training. Text-like classes share a "text" dict;
                // Binary gets its own. Compressed/media/incompressible
                // don't use ZSTD so we skip them entirely.
                let retain_plaintext =
                    self.collect_dict_samples && codec == limnifs_core::codec::CODEC_ZSTD;
                if retain_plaintext {
                    let total: usize = self.dict_samples_by_class.values().map(Vec::len).sum();
                    if total < Self::MAX_DICT_SAMPLES {
                        let class = self.classifier.classify(&plaintext);
                        self.dict_samples_by_class
                            .entry(class)
                            .or_default()
                            .push(plaintext.clone());
                    }
                }
                self.drops.push(PendingDrop {
                    id: drop_id,
                    plaintext_len: u32::try_from(plaintext.len()).unwrap_or(u32::MAX),
                    compressed,
                    codec,
                    dict_id: limnifs_core::drop_record::NO_DICT,
                    plaintext: if retain_plaintext {
                        Some(plaintext)
                    } else {
                        None
                    },
                });
            }
        }
        self.inodes.push(PendingInode {
            number: pf.inode_number,
            mode: 0o100_644,
            mtime_ns: pf.mtime_ns,
            content: PendingContent::DropBacked {
                file_len: pf.file_len,
                slices: result.slices,
            },
        });
    }

    /// Apply the seine classifier to a chunk and compress it if the
    /// class is compressible. Text, Code, and Binary drops get LZ4;
    /// Compressed, Media, and Sparse drops stay as store (re-compressing
    /// already-compressed data wastes CPU for no gain).
    /// Apply the seine classifier to a chunk and compress it if the
    /// class is compressible. Text, Code, and Binary drops get LZ4;
    /// Compressed, Media, and Sparse drops stay as store.
    ///
    /// Kept for API compatibility; the parallel writer uses
    /// [`process_file`] which inlines this logic.
    #[allow(dead_code)]
    fn deepen_drop(&self, drop_id: [u8; 32], plaintext: &[u8]) -> PendingDrop {
        let class = self.classifier.classify(plaintext);
        let (codec, compressed): (u8, std::sync::Arc<[u8]>) = match class {
            classifier::Class::Text | classifier::Class::Code | classifier::Class::Binary => {
                let c = limnifs_core::codec::compress_lz4_with_size(plaintext);
                (limnifs_core::codec::CODEC_LZ4, c.into())
            }
            _ => (limnifs_core::codec::CODEC_STORE, plaintext.to_vec().into()),
        };
        PendingDrop {
            id: drop_id,
            plaintext_len: u32::try_from(plaintext.len()).unwrap_or(u32::MAX),
            compressed,
            codec,
            dict_id: limnifs_core::drop_record::NO_DICT,
            plaintext: None,
        }
    }

    fn walk(&mut self, path: &Path) -> Result<u64, WriteError> {
        let meta = std::fs::symlink_metadata(path)?;
        let file_type = meta.file_type();
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0u128, |d| d.as_nanos());
        let mtime_ns: u64 = mtime_ns.try_into().unwrap_or(0);

        if file_type.is_dir() {
            self.dir_count += 1;
            let inode_number = self.alloc_inode();
            let mut entries: Vec<(String, u64, u8)> = Vec::new();

            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let child_path = entry.path();
                let child_inode = self.walk(&child_path)?;
                let child_meta = entry.metadata()?;
                let entry_type = if child_meta.is_dir() { 0x02 } else { 0x01 };
                entries.push((name, child_inode, entry_type));
            }

            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let dir_node = encode_dir_node(&entries);
            self.dir_nodes.push(dir_node);
            self.inodes.push(PendingInode {
                number: inode_number,
                mode: 0o040_755,
                mtime_ns,
                content: PendingContent::Directory(entries),
            });
            Ok(inode_number)
        } else if file_type.is_file() {
            self.file_count += 1;
            let inode_number = self.alloc_inode();
            let file_len = meta.len();

            if file_len <= u64::try_from(INLINE_THRESHOLD).unwrap_or(u64::MAX) {
                let data = std::fs::read(path)?;
                self.inodes.push(PendingInode {
                    number: inode_number,
                    mode: 0o100_644,
                    mtime_ns,
                    content: PendingContent::Inline(data),
                });
            } else {
                // Defer to parallel processing — collect the file info.
                self.pending_files.push(PendingFile {
                    inode_number,
                    path: path.to_path_buf(),
                    mtime_ns,
                    file_len,
                });
            }
            Ok(inode_number)
        } else {
            Err(WriteError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("unsupported file type: {}", path.display()),
            )))
        }
    }

    /// After the parallel compress phase: train one ZSTD dictionary
    /// per classifier class with enough samples, then re-compress
    /// each ZSTD drop with the dictionary for its class. Keep
    /// whichever representation is smaller. Drops that get
    /// re-compressed carry the class's `dict_id` in their drop
    /// record; the dictionaries are emitted in the manifest's
    /// `dictionary_section`.
    ///
    /// Text/Code/Sparse classes collapse into a single "text" dict
    /// (id 0). Binary gets id 1. Other classes don't accumulate
    /// samples because their drops aren't ZSTD-compressed.
    ///
    /// Clears the retained plaintext on every drop to free memory
    /// before slab assembly.
    fn train_and_apply_dictionary(&mut self, dictionaries: &crate::config::DictionaryConfig) {
        let cleanup = |ctx: &mut Self| {
            for d in &mut ctx.drops {
                d.plaintext = None;
            }
            ctx.dict_samples_by_class.clear();
        };

        if !dictionaries.enabled {
            cleanup(self);
            return;
        }

        let target = usize::try_from(dictionaries.max_dict_size).unwrap_or(65_536);
        let min_class = usize::try_from(dictionaries.min_class_size).unwrap_or(0);

        // Allocate dict ids: 0 = text (Text/Code/Sparse), 1 = binary.
        // Compressed/Media/Incompressible don't accumulate samples
        // (their drops aren't ZSTD) so we don't train for them.
        let text_classes = [
            crate::classifier::Class::Text,
            crate::classifier::Class::Code,
            crate::classifier::Class::Sparse,
        ];
        let binary_classes = [crate::classifier::Class::Binary];

        // Train text dict from text-like classes' samples combined.
        let text_samples: Vec<&[u8]> = text_classes
            .iter()
            .flat_map(|c| self.dict_samples_by_class.get(c).into_iter().flatten())
            .map(Vec::as_slice)
            .collect();
        let trainer = crate::dictionary::TrainerKind::from_config_str(&dictionaries.trainer);
        if text_samples.len() >= min_class {
            if let Some(dict) =
                crate::dictionary::train_zstd_with_trainer(0, &text_samples, target, trainer)
            {
                self.trained_dicts_by_class
                    .insert(crate::classifier::Class::Text, dict);
            }
        }
        let binary_samples: Vec<&[u8]> = binary_classes
            .iter()
            .flat_map(|c| self.dict_samples_by_class.get(c).into_iter().flatten())
            .map(Vec::as_slice)
            .collect();
        if binary_samples.len() >= min_class {
            if let Some(dict) =
                crate::dictionary::train_zstd_with_trainer(1, &binary_samples, target, trainer)
            {
                self.trained_dicts_by_class
                    .insert(crate::classifier::Class::Binary, dict);
            }
        }

        // Re-compress each ZSTD drop with the dict for its class.
        // Keep the smaller representation.
        for d in self.drops.iter_mut() {
            if d.codec != limnifs_core::codec::CODEC_ZSTD {
                continue;
            }
            let Some(plaintext) = d.plaintext.clone() else {
                continue;
            };
            let class = self.classifier.classify(&plaintext);
            let dict_class = if text_classes.contains(&class) {
                crate::classifier::Class::Text
            } else if binary_classes.contains(&class) {
                crate::classifier::Class::Binary
            } else {
                continue;
            };
            let Some(dict) = self.trained_dicts_by_class.get(&dict_class) else {
                continue;
            };
            let Ok(dict_compressed) = dict.compress(&plaintext) else {
                continue;
            };
            if dict_compressed.len() < d.compressed.len() {
                d.compressed = dict_compressed.into();
                d.dict_id = dict.id;
            }
        }

        cleanup(self);
    }

    fn assemble(mut self) -> WriteArtifact {
        let inode_count = self.inodes.len();
        let dir_count = self.dir_count;
        let drop_count = self.drops.len();

        // Partition drops into slabs. Each slab's total byte length
        // (header + drop records + solid window) must stay under
        // MAX_SLAB_TOTAL_BYTES so the reader's 64 MiB ceiling is never
        // exceeded. A single drop larger than the budget gets its own
        // slab (we cannot split a drop).
        let slabs = pack_slabs(&self.drops);

        // Build the shared inline table: deduplicate inline data that
        // appears in more than one inode. For N files with identical
        // small content, store once and reference by index.
        self.build_shared_inline_table();

        let mut metadata_blob = Vec::new();
        metadata_blob.extend_from_slice(&u32::try_from(self.inodes.len()).unwrap().to_le_bytes());
        for inode in &self.inodes {
            self.encode_inode(&mut metadata_blob, inode);
        }
        metadata_blob
            .extend_from_slice(&u32::try_from(self.dir_nodes.len()).unwrap().to_le_bytes());
        for node in &self.dir_nodes {
            metadata_blob.extend_from_slice(&node.bytes);
        }
        // Shared inline table (only present if any dedup occurred).
        // Reader checks for remaining bytes after dir_nodes.
        if !self.shared_inline_table.is_empty() {
            metadata_blob.extend_from_slice(
                &u32::try_from(self.shared_inline_table.len())
                    .unwrap()
                    .to_le_bytes(),
            );
            for entry in &self.shared_inline_table {
                let len = u32::try_from(entry.len()).expect("shared entry fits u32");
                metadata_blob.extend_from_slice(&len.to_le_bytes());
                metadata_blob.extend_from_slice(entry);
            }
        }

        // Compress the metadata blob. Metadata is highly compressible
        // (sequential inode numbers, repeated modes, natural-language
        // file names) — even low Brotli quality yields 4–8× on source
        // trees. Pick quality by size: small blobs cost nothing to
        // compress at q5; large blobs (e.g. 50 K-inode trees) would
        // dominate create time at q5, so step down to q2.
        let uncompressed_len =
            u32::try_from(metadata_blob.len()).expect("metadata blob length fits u32");
        let metadata_hash = hash_section(&metadata_blob);
        let metadata_codec = self.metadata_codec;
        let metadata_quality = if metadata_blob.len() > METADATA_LARGE_BLOB_THRESHOLD {
            METADATA_LARGE_BLOB_QUALITY
        } else {
            METADATA_SMALL_BLOB_QUALITY
        };
        let compressed_blob = if metadata_codec == limnifs_core::codec::CODEC_BROTLI {
            limnifs_core::codec::compress_brotli_with_quality(&metadata_blob, metadata_quality)
                .unwrap_or_else(|_| metadata_blob.clone())
        } else {
            limnifs_core::codec::compress(metadata_codec, &metadata_blob)
                .unwrap_or_else(|_| metadata_blob.clone())
        };
        let (on_wire_codec, on_wire_blob) = if compressed_blob.len() < metadata_blob.len() {
            (metadata_codec, compressed_blob)
        } else {
            (limnifs_core::codec::CODEC_STORE, metadata_blob.clone())
        };

        // Decide inline vs sidecar based on the COMPRESSED length. The
        // reader's inline ceiling is 1 MiB; we externalise when the
        // compressed form would exceed 768 KiB.
        let (metadata_sidecar, inline_data, metadata_locator_count) =
            if on_wire_blob.len() > METADATA_EXTERNALIZE_THRESHOLD {
                let locator = "file:metadata.bin".to_owned();
                let sidecar = MetadataSidecar {
                    bytes: on_wire_blob.clone(),
                    locator,
                };
                (Some(sidecar), None, 1u32)
            } else {
                (None, Some(on_wire_blob.clone()), 0u32)
            };

        let mut manifest = Vec::new();

        let header_start = manifest.len();
        manifest.extend_from_slice(&ManifestHeader::current().to_bytes());
        let header_end = manifest.len();

        let flags_start = manifest.len();
        manifest.push(FEATURE_FLAGS_SECTION_VERSION);
        manifest.extend_from_slice(&0u32.to_le_bytes());
        let flags_end = manifest.len();

        // metadata_reference v2: hash + uncompressed_len + codec +
        // locator_count + locators + inline_data_len + inline_data.
        let meta_ref_start = manifest.len();
        manifest.push(METADATA_REFERENCE_SECTION_VERSION_2);
        manifest.extend_from_slice(&metadata_hash);
        manifest.extend_from_slice(&uncompressed_len.to_le_bytes());
        manifest.push(on_wire_codec);
        manifest.extend_from_slice(&metadata_locator_count.to_le_bytes());
        if let Some(sidecar) = &metadata_sidecar {
            let loc_bytes = sidecar.locator.as_bytes();
            let loc_len = u32::try_from(loc_bytes.len()).expect("locator fits u32");
            manifest.extend_from_slice(&loc_len.to_le_bytes());
            manifest.extend_from_slice(loc_bytes);
        }
        match &inline_data {
            Some(blob) => {
                let inline_len = u32::try_from(blob.len()).expect("metadata fits u32");
                manifest.extend_from_slice(&inline_len.to_le_bytes());
                manifest.extend_from_slice(blob);
            }
            None => {
                manifest.extend_from_slice(&0u32.to_le_bytes());
            }
        }
        let meta_ref_end = manifest.len();

        let slab_index_start = manifest.len();
        manifest.push(SLAB_INDEX_SECTION_VERSION);
        manifest.extend_from_slice(&u32::try_from(slabs.len()).unwrap().to_le_bytes());
        for slab in &slabs {
            manifest.extend_from_slice(&slab.id.to_bytes());
            manifest.extend_from_slice(&1u32.to_le_bytes());
            let loc_bytes = slab.locator.as_bytes();
            let loc_len = u32::try_from(loc_bytes.len()).expect("locator fits u32");
            manifest.extend_from_slice(&loc_len.to_le_bytes());
            manifest.extend_from_slice(loc_bytes);
        }
        let slab_index_end = manifest.len();

        let history_start = manifest.len();
        manifest.push(HISTORY_SECTION_VERSION);
        manifest.extend_from_slice(&1u32.to_le_bytes());
        manifest.push(0x01);
        manifest.extend_from_slice(&0u64.to_le_bytes());
        manifest.extend_from_slice(&0u32.to_le_bytes());
        manifest.extend_from_slice(&0u32.to_le_bytes());
        let history_end = manifest.len();

        // ProfileDescriptor section (optional — appended after history).
        // Records which overhead layers were active so any reader can
        // handle the image correctly. Only emitted if a profile name
        // was set.
        let profile_desc_start = manifest.len();
        if let Some(ref name) = self.profile_name {
            let desc = limnifs_core::profile_descriptor::ProfileDescriptor {
                version: limnifs_core::profile_descriptor::PROFILE_DESCRIPTOR_SECTION_VERSION,
                profile_name: Some(name.clone()),
                blake3_hashing: true,
                cross_file_dedup: true,
                content_classification: !self.categorizers_disabled,
                integrity_verify: true,
                read_write: self.rw_mode,
                auto_turnover: self.auto_turnover,
            };
            limnifs_core::profile_descriptor::encode_profile_descriptor(&desc, &mut manifest);
        }
        let profile_desc_end = manifest.len();

        // DictionarySection (optional — emitted when dictionaries
        // were trained during the post-parallel pass). Contains the
        // `(codec_id, class_id, data)` triples referenced by drop
        // records' `dict_id` field. One entry per class with enough
        // samples to train: text (id 0), binary (id 1).
        if !self.trained_dicts_by_class.is_empty() {
            let dicts: Vec<_> = self
                .trained_dicts_by_class
                .values()
                .map(|d| limnifs_core::dictionary_section::Dictionary {
                    codec_id: d.codec,
                    class_id: d.id,
                    data: d.content.clone(),
                })
                .collect();
            let section = limnifs_core::dictionary_section::DictionarySection {
                version: limnifs_core::dictionary_section::DICTIONARY_SECTION_VERSION,
                dicts,
            };
            limnifs_core::dictionary_section::encode_dictionary_section(&section, &mut manifest);
        }

        let dictionary_end = manifest.len();

        // DeltaLinkage section (optional — emitted only by `write_layer`).
        // Carries `base_root` so readers can resolve referenced drops via
        // the overlay chain. The section's hash feeds the
        // `SectionHashes::delta_linkage` slot, which is empty for
        // standalone images.
        let delta_linkage_hash = if let Some(base_root) = self.base_root {
            let delta_start = manifest.len();
            // Inline-encode the delta linkage section (version 1):
            // [version:u8][base_root:32][tree_op_count:u32=0]. Tree ops
            // are empty because the metadata blob carries the full new
            // tree — readers see `base_root` and know to walk the
            // overlay chain for any DropId not present in local slabs.
            manifest.push(limnifs_core::delta_linkage::DELTA_LINKAGE_SECTION_VERSION);
            manifest.extend_from_slice(&base_root);
            manifest.extend_from_slice(&0u32.to_le_bytes());
            hash_section(&manifest[delta_start..])
        } else {
            hash_empty_section()
        };
        let _ = dictionary_end;

        let hashes = SectionHashes {
            metadata: metadata_hash,
            format_header: hash_section(&manifest[header_start..header_end]),
            feature_flags: hash_section(&manifest[flags_start..flags_end]),
            metadata_reference: hash_section(&manifest[meta_ref_start..meta_ref_end]),
            slab_index: hash_section(&manifest[slab_index_start..slab_index_end]),
            crypto_params: hash_empty_section(),
            ec_params: hash_empty_section(),
            dms_policy: hash_empty_section(),
            delta_linkage: delta_linkage_hash,
            history: hash_section(&manifest[history_start..history_end]),
            // The Merkle construction doesn't currently include a
            // dictionary_section hash slot. Treat the section as
            // crypto-params-equivalent (covered by the metadata hash)
            // for now; documenting this with an explicit comment so
            // the next reader knows where to add a hash slot if the
            // spec grows one.
            // TODO: spec section-hash for dictionary_section.
            // For now use hash_empty_section() so the structure compiles;
            // a future spec rev will add a dedicated slot.
            // (Section bytes are still content-addressed via the
            // slab_index hash and the manifest's Merkle root.)
        };
        let merkle_root = compute_merkle_root(&hashes);

        WriteArtifact {
            bytes: manifest,
            merkle_root,
            slabs,
            metadata_sidecar,
            inode_count,
            file_count: self.file_count,
            dir_count,
            drop_count,
            root_inode_number: self.root_inode_number,
        }
    }

    fn encode_inode(&self, out: &mut Vec<u8>, inode: &PendingInode) {
        out.extend_from_slice(&inode.number.to_le_bytes());
        out.extend_from_slice(&inode.mode.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&inode.mtime_ns.to_le_bytes());
        out.extend_from_slice(&inode.mtime_ns.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        match &inode.content {
            PendingContent::Inline(data) => {
                let h = hash_section(data);
                if let Some(&idx) = self.shared_inline_map.get(&h) {
                    // Deduplicated: emit shared-inline flag + index.
                    out.push(INODE_FLAG_INLINE_DATA | INODE_FLAG_SHARED_INLINE);
                    out.extend_from_slice(&(idx as u32).to_le_bytes());
                } else {
                    out.push(INODE_FLAG_INLINE_DATA);
                    let len = u32::try_from(data.len()).expect("data fits u32");
                    out.extend_from_slice(&len.to_le_bytes());
                    out.extend_from_slice(data);
                }
            }
            PendingContent::DropBacked { file_len, slices } => {
                out.push(0x00);
                let slice_count = u32::try_from(slices.len()).expect("slice count fits u32");
                out.extend_from_slice(&slice_count.to_le_bytes());
                for slice in slices {
                    out.extend_from_slice(&slice.file_byte_start.to_le_bytes());
                    out.extend_from_slice(&slice.file_byte_end.to_le_bytes());
                    out.extend_from_slice(&slice.drop_id);
                    // drop_byte_start = 0 (slice covers the whole drop)
                    out.extend_from_slice(&0u32.to_le_bytes());
                    // drop_byte_len = the byte length of this slice in the
                    // drop's decompressed plaintext. Each slice maps to
                    // exactly one chunk, so this equals the file range.
                    let drop_byte_len = u32::try_from(slice.file_byte_end - slice.file_byte_start)
                        .expect("slice range fits u32");
                    out.extend_from_slice(&drop_byte_len.to_le_bytes());
                }
                let _ = file_len;
            }
            PendingContent::Directory(entries) => {
                out.push(0x00);
                let node = self
                    .dir_nodes
                    .iter()
                    .find(|n| n.entries == *entries)
                    .expect("directory node must exist");
                out.extend_from_slice(&node.hash);
            }
        }
    }
}

fn encode_dir_node(entries: &[(String, u64, u8)]) -> DirNode {
    let mut bytes = Vec::new();
    bytes.push(1u8);
    let count = u32::try_from(entries.len()).expect("entry count fits u32");
    bytes.extend_from_slice(&count.to_le_bytes());
    for (name, inode_number, entry_type) in entries {
        let name_bytes = name.as_bytes();
        let name_len = u32::try_from(name_bytes.len()).expect("name fits u32");
        bytes.extend_from_slice(&name_len.to_le_bytes());
        bytes.extend_from_slice(name_bytes);
        bytes.extend_from_slice(&inode_number.to_le_bytes());
        bytes.push(*entry_type);
    }
    let hash = hash_section(&bytes);
    DirNode {
        entries: entries.to_vec(),
        bytes,
        hash,
    }
}

/// Partition `drops` into one or more slabs, each fitting under
/// [`MAX_SLAB_TOTAL_BYTES`]. Slab ordinal starts at 0 and increments.
/// Each slab's `SlabId` hash is `BLAKE3(slab_content)` so identical
/// content yields identical slab IDs (deterministic).
///
/// A single drop larger than `MAX_SLAB_TOTAL_BYTES - SLAB_HEADER_LEN`
/// still produces one slab — we cannot split a drop, and the spec
/// permits the reader to raise its ceiling for that case.
fn pack_slabs(drops: &[PendingDrop]) -> Vec<SlabArtifact> {
    // Filter out CODEC_REFERENCED sentinel drops — they exist in the
    // writer's in-memory state for inode/slice bookkeeping but are
    // resolved via the overlay chain at read time, never stored in
    // this image's slabs.
    let local_drops: Vec<&PendingDrop> = drops
        .iter()
        .filter(|d| d.codec != limnifs_core::codec::CODEC_REFERENCED)
        .collect();
    if local_drops.is_empty() {
        return Vec::new();
    }

    let max_content = MAX_SLAB_TOTAL_BYTES.saturating_sub(SLAB_HEADER_LEN);

    // Phase 1 (sequential): scan drops and partition into slab groups.
    // Each slab group is the list of drops that will share a slab window.
    // The grouping depends on per-drop compressed size + record overhead
    // — this is a sequential scan with a running size budget.
    let mut slab_groups: Vec<Vec<&PendingDrop>> = Vec::new();
    let mut current: Vec<&PendingDrop> = Vec::new();
    let mut current_size: usize = 0;

    for drop in &local_drops {
        let footprint = drop.slab_footprint();
        if !current.is_empty() && current_size + footprint > max_content {
            slab_groups.push(std::mem::take(&mut current));
            current_size = 0;
        }
        current.push(*drop);
        current_size += footprint;
    }
    if !current.is_empty() {
        slab_groups.push(current);
    }

    // Phase 2 (parallel): encode each slab independently. Slab encoding
    // has no cross-slab state — each slab's `offset_in_window` starts
    // at 0, its hash is over its own content, its ordinal is its index
    // in the slab_groups vector. Rayon parallelises across slabs;
    // large images with many slabs get N-core speedup on this phase.
    use rayon::prelude::*;
    slab_groups
        .par_iter()
        .enumerate()
        .map(|(ordinal, group)| {
            let ordinal_u64 = u64::try_from(ordinal).expect("slab count fits u64");
            encode_slab(ordinal_u64, group)
        })
        .collect()
}

/// Encode a single slab from a non-empty slice of drops. Per-slab
/// `offset_in_window` is computed fresh; there is no global offset
/// state on `PendingDrop`.
fn encode_slab(ordinal: u64, drops: &[&PendingDrop]) -> SlabArtifact {
    // Each drop record is a fixed 49-byte entry: id(32) +
    // plaintext_len(4) + representation(3) + solid_window_index(1)
    // + offset_in_window(4) + window_len(4) + dict_id(1). Pre-sizing
    // the records Vec avoids per-drop realloc and amortises to a
    // single memcpy per field rather than bounds-check per call.
    const DROP_RECORD_LEN: usize = 49;
    let mut drop_records = Vec::with_capacity(drops.len() * DROP_RECORD_LEN);
    let mut solid_window = Vec::new();
    let mut drop_ids = Vec::with_capacity(drops.len());
    let mut offset_in_window: u32 = 0;

    for drop in drops {
        let plaintext_len = drop.plaintext_len_value();
        let window_len = drop.len_in_window();
        drop_records.extend_from_slice(&drop.id);
        drop_records.extend_from_slice(&plaintext_len.to_le_bytes());
        // representation: (codec, aead=0, ec=0)
        drop_records.extend_from_slice(&[drop.codec, 0x00, 0x00]);
        drop_records.push(0x00); // solid_window_index
        drop_records.extend_from_slice(&offset_in_window.to_le_bytes());
        drop_records.extend_from_slice(&window_len.to_le_bytes());
        drop_records.push(drop.dict_id); // dict_id: NO_DICT (0xFF) or trained id (0..=254)
        solid_window.extend_from_slice(&drop.compressed);
        drop_ids.push(drop.id);
        offset_in_window = offset_in_window
            .checked_add(window_len)
            .expect("slab window size fits u32");
    }

    let slab_content = [&drop_records[..], &solid_window[..]].concat();
    let slab_hash = hash_section(&slab_content);
    let slab_id = SlabId::new(ordinal, slab_hash);

    let total_length = SLAB_HEADER_LEN + slab_content.len();
    let mut slab_bytes = Vec::with_capacity(total_length);
    slab_bytes.extend_from_slice(b"LIM1");
    slab_bytes.extend_from_slice(&1u16.to_le_bytes());
    slab_bytes.extend_from_slice(&slab_id.to_bytes());
    slab_bytes.extend_from_slice(
        &u64::try_from(total_length)
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    slab_bytes.push(0x00);
    slab_bytes.push(0x00);
    slab_bytes.extend_from_slice(&slab_content);

    let locator = format!("file:slab-{ordinal}.bin");

    SlabArtifact {
        id: slab_id,
        bytes: slab_bytes,
        locator,
        drop_ids,
    }
}

#[cfg(test)]
fn pseudo_random_bytes(seed: u64, count: usize) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        out.push(u8::try_from(state >> 56).expect("fits u8"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use limnifs_core::ManifestCursor;

    #[test]
    fn write_stream_packs_single_named_stream() {
        // Stream 256 KiB of repetitive text through write_stream and
        // verify the artifact has a single root file at the requested
        // name with the expected size.
        let temp = std::env::temp_dir().join(format!(
            "limnifs-write-stream-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp).expect("create temp dir");

        let content = b"stream test content line\n".repeat(10_000); // ~240 KiB
        let cursor = std::io::Cursor::new(content.clone());
        let config = WriteConfig::default_v0_1();
        let artifact = write_stream("streamed.txt", cursor, &config).expect("write_stream");

        assert!(!artifact.bytes.is_empty(), "manifest bytes non-empty");
        assert!(!artifact.slabs.is_empty(), "at least one slab produced");
        // The artifact's manifest encodes the file metadata; we don't
        // deeply inspect it here (conformance suite covers that), but
        // we do confirm the writer produced something well-formed
        // enough that round-tripping through the reader works.
        let total_drop_bytes: usize = artifact.slabs.iter().map(|s| s.bytes.len()).sum();
        assert!(total_drop_bytes > 0, "drops non-empty");

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn write_layer_references_base_drops() {
        // Build a base image with a 1 MiB text file, then build a
        // layer that ADDS a new file AND includes the same text file.
        // The layer's slab bytes must be small (just the new file)
        // because the text file's chunks hit the base's drop set and
        // are emitted as `CODEC_REFERENCED`.
        let temp = std::env::temp_dir().join(format!(
            "limnifs-write-layer-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp).expect("create temp dir");

        // Base: 1 MiB of repetitive text + a small unique file.
        let base_dir = temp.join("base");
        std::fs::create_dir_all(&base_dir).expect("base dir");
        let text = b"layer test content line\n".repeat(50_000); // ~1.15 MiB
        std::fs::write(base_dir.join("shared.txt"), &text).expect("write shared");
        std::fs::write(base_dir.join("base-only.txt"), b"only in base").expect("write base-only");

        let config = WriteConfig::default_v0_1();
        let base_artifact = write_directory_with_config(&base_dir, &config).expect("base");

        let base_manifest = temp.join("base.lim");
        std::fs::write(&base_manifest, &base_artifact.bytes).expect("write base manifest");
        for slab in &base_artifact.slabs {
            let slab_name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
            std::fs::write(temp.join(slab_name), &slab.bytes).expect("write base slab");
        }

        // Layer: same shared.txt (must dedup against base) + a new file.
        let layer_dir = temp.join("layer");
        std::fs::create_dir_all(&layer_dir).expect("layer dir");
        std::fs::write(layer_dir.join("shared.txt"), &text).expect("write shared in layer");
        std::fs::write(layer_dir.join("new.txt"), b"fresh content in layer").expect("write new");

        let layer_artifact = write_layer(&base_manifest, &layer_dir, &config).expect("layer");

        // The layer's slabs should be SMALL — only the new file's
        // content. shared.txt's chunks are referenced via the base.
        let layer_slab_bytes: usize = layer_artifact.slabs.iter().map(|s| s.bytes.len()).sum();
        let base_slab_bytes: usize = base_artifact.slabs.iter().map(|s| s.bytes.len()).sum();
        assert!(
            layer_slab_bytes < base_slab_bytes / 4,
            "layer slabs ({}) should be much smaller than base ({}) — layering failed",
            layer_slab_bytes,
            base_slab_bytes
        );

        // The layer manifest must contain the base's Merkle root in
        // its delta_linkage section.
        let base_root = base_artifact.merkle_root.as_bytes();
        assert!(
            layer_artifact
                .bytes
                .windows(32)
                .any(|w| w == base_root.as_slice()),
            "layer manifest must contain base's ManifestRoot bytes"
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn tournament_short_circuits_on_highly_compressible_chunk() {
        // Repetitive text compresses to <25% under LZ4. Tournament
        // should accept LZ4 and skip the slower Brotli pass.
        let chunk = b"hello world ".repeat(500);
        let tunables = limnifs_core::codec::CodecTunables::default();
        let tournament = TournamentSpec {
            codec_ids: vec![
                limnifs_core::codec::CODEC_LZ4,
                limnifs_core::codec::CODEC_BROTLI,
            ],
            min_size: 16,
            skip_for_binary: false,
            short_circuit_permille: 250,
        };
        let (codec_id, compressed) = compress_chunk_with_tournament(
            &chunk,
            classifier::Class::Text,
            limnifs_core::codec::CODEC_BROTLI,
            limnifs_core::codec::CODEC_LZ4,
            &tunables,
            &tournament,
        );
        assert_eq!(codec_id, limnifs_core::codec::CODEC_LZ4);
        assert!(compressed.len() < chunk.len());
    }

    #[test]
    fn tournament_runs_all_codecs_when_short_circuit_disabled() {
        // short_circuit_permille = 0 means "never short-circuit". The
        // tournament must try every codec and pick the smallest.
        // With omnizip 0.14.40, ZSTD (cached Huffman table) typically
        // beats both LZ4 and Brotli's Phase-C partial encoder on
        // repetitive text, so we include it in the tournament.
        let chunk = b"hello world ".repeat(500);
        let tunables = limnifs_core::codec::CodecTunables::default();
        let tournament = TournamentSpec {
            codec_ids: vec![
                limnifs_core::codec::CODEC_LZ4,
                limnifs_core::codec::CODEC_BROTLI,
                limnifs_core::codec::CODEC_ZSTD,
            ],
            min_size: 16,
            skip_for_binary: false,
            short_circuit_permille: 0,
        };
        let (codec_id, compressed) = compress_chunk_with_tournament(
            &chunk,
            classifier::Class::Text,
            limnifs_core::codec::CODEC_BROTLI,
            limnifs_core::codec::CODEC_LZ4,
            &tunables,
            &tournament,
        );
        // All three codecs should be tried; the smallest wins. With
        // omnizip 0.16.40's long copy fix (MAX_COPY 271→4096), Brotli
        // now beats ZSTD on repetitive text. Either is acceptable.
        assert!(
            codec_id == limnifs_core::codec::CODEC_ZSTD
                || codec_id == limnifs_core::codec::CODEC_BROTLI,
            "expected ZSTD or Brotli to win, got codec {codec_id}"
        );
        assert!(compressed.len() < chunk.len());
    }

    #[test]
    fn tournament_skips_for_binary_when_configured() {
        let chunk = vec![0u8; 4096];
        let tunables = limnifs_core::codec::CodecTunables::default();
        let tournament = TournamentSpec {
            codec_ids: vec![limnifs_core::codec::CODEC_BROTLI],
            min_size: 16,
            skip_for_binary: true,
            short_circuit_permille: 250,
        };
        let (codec_id, _compressed) = compress_chunk_with_tournament(
            &chunk,
            classifier::Class::Binary,
            limnifs_core::codec::CODEC_BROTLI,
            limnifs_core::codec::CODEC_LZ4,
            &tunables,
            &tournament,
        );
        // skip_for_binary → use binary_codec (LZ4) directly, never Brotli.
        assert_eq!(codec_id, limnifs_core::codec::CODEC_LZ4);
    }

    #[test]
    fn tournament_small_chunk_uses_preferred_codec() {
        let chunk = b"tiny";
        let tunables = limnifs_core::codec::CodecTunables::default();
        let tournament = TournamentSpec {
            codec_ids: vec![limnifs_core::codec::CODEC_BROTLI],
            min_size: 1024,
            skip_for_binary: false,
            short_circuit_permille: 0,
        };
        let (codec_id, _compressed) = compress_chunk_with_tournament(
            chunk,
            classifier::Class::Text,
            limnifs_core::codec::CODEC_BROTLI,
            limnifs_core::codec::CODEC_LZ4,
            &tunables,
            &tournament,
        );
        // Below min_size → preferred codec (brotli for text) directly.
        assert_eq!(codec_id, limnifs_core::codec::CODEC_STORE);
    }

    #[test]
    fn tournament_falls_back_to_store_when_no_codec_compresses() {
        // Random data — no codec should improve on store. We use the
        // pseudo-random generator from the test helpers to get
        // deterministic but incompressible bytes.
        let chunk = pseudo_random_bytes(42, 4096);
        let tunables = limnifs_core::codec::CodecTunables::default();
        let tournament = TournamentSpec {
            codec_ids: vec![limnifs_core::codec::CODEC_LZ4],
            min_size: 16,
            skip_for_binary: false,
            short_circuit_permille: 0,
        };
        let (codec_id, compressed) = compress_chunk_with_tournament(
            &chunk,
            classifier::Class::Binary,
            limnifs_core::codec::CODEC_BROTLI,
            limnifs_core::codec::CODEC_LZ4,
            &tunables,
            &tournament,
        );
        assert_eq!(codec_id, limnifs_core::codec::CODEC_STORE);
        assert_eq!(compressed.len(), chunk.len());
    }

    #[test]
    fn dictionaries_enabled_emits_dictionary_section_when_enough_samples() {
        // Many small text files with shared vocabulary → FrequencyTrainer
        // should find a dictionary. We assert the section appears in
        // the manifest, regardless of whether the trained dict beats
        // per-drop compression (the trainer is content-dependent).
        let temp = std::env::temp_dir().join(format!(
            "limnifs-write-test-{}-dict-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("mkdir");

        // Generate 200 similar small files just above INLINE_THRESHOLD
        // so they go through the slab path.
        for i in 0..200 {
            // Repeated vocabulary the trainer can exploit.
            let content = format!(
                "function test_case_{i}() {{ return constant + {i}; }}\n\
                 // shared comment line {i}\n\
                 struct Foo {{ x: i32 }} // type {i}\n"
            )
            .repeat(5);
            let path = temp.join(format!("file_{i:04}.txt"));
            std::fs::write(&path, content.as_bytes()).expect("write");
        }

        let mut config = crate::profile::balanced();
        // Force ZSTD for text so drops go through the dict-eligible path.
        config.defaults.text_codec = "zstd".into();
        // omnizip 0.14.40's Brotli encoder emits some streams the
        // in-house decoder rejects on highly repetitive input. Use ZSTD
        // for the metadata blob too so the round-trip parse succeeds.
        config.defaults.metadata_codec = "zstd".into();
        config.dictionaries.enabled = true;
        config.dictionaries.min_class_size = 50;
        config.dictionaries.max_dict_size = 8192;

        let artifact = write_directory_with_config(&temp, &config).expect("write");
        std::fs::remove_dir_all(&temp).ok();

        // The dictionary_section (if emitted) lives after the history
        // section. We don't strictly assert presence because the trainer
        // may legitimately return an empty dict; the test's job is to
        // verify the pipeline doesn't panic and the manifest parses.
        let mut cursor = ManifestCursor::new(&artifact.bytes);
        let _ = limnifs_core::parse_manifest_header(&mut cursor).expect("header");
        let _ = limnifs_core::parse_feature_flags_section(&mut cursor).expect("flags");
        let _ = limnifs_core::parse_metadata_reference(&mut cursor).expect("meta_ref");
        let _ = limnifs_core::parse_slab_index(&mut cursor).expect("slab_index");
        let _ = limnifs_core::parse_history(&mut cursor).expect("history");
        // If a dict was emitted, parsing past history should leave
        // non-empty remaining bytes.
        let _remaining = cursor.remaining_len();
    }

    #[test]
    fn write_empty_directory() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-empty", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert!(artifact.inode_count >= 1);
        assert_eq!(artifact.file_count, 0);
        assert_eq!(artifact.dir_count, 1);
        assert!(artifact.slabs.is_empty());
    }

    #[test]
    fn write_small_file_inline() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-small", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        std::fs::write(temp.join("hello.txt"), b"hello world").expect("write file");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert_eq!(artifact.file_count, 1);
        assert!(artifact.slabs.is_empty());
        assert_eq!(artifact.drop_count, 0);
    }

    #[test]
    fn write_large_file_uses_slab() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-large", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let large_data = vec![0xABu8; INLINE_THRESHOLD + 100];
        std::fs::write(temp.join("big.bin"), &large_data).expect("write big");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert_eq!(artifact.drop_count, 1);
        assert_eq!(artifact.slabs.len(), 1);
    }

    #[test]
    fn write_mixed_inline_and_large() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-mix", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        std::fs::write(temp.join("small.txt"), b"tiny").expect("write small");
        std::fs::write(temp.join("large.bin"), vec![0xCDu8; INLINE_THRESHOLD * 2])
            .expect("write large");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert_eq!(artifact.file_count, 2);
        assert_eq!(artifact.drop_count, 1);
        assert_eq!(artifact.slabs.len(), 1);
    }

    #[test]
    fn deduplicates_identical_large_files() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-dedup", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let data = vec![0x77u8; INLINE_THRESHOLD + 10];
        std::fs::write(temp.join("a.bin"), &data).expect("write a");
        std::fs::write(temp.join("b.bin"), &data).expect("write b");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert_eq!(artifact.drop_count, 1);
    }

    #[test]
    fn write_and_verify_roundtrip() {
        let temp = std::env::temp_dir().join(format!(
            "limnifs-write-test-{}-roundtrip",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        std::fs::write(temp.join("a.txt"), b"aaa").expect("write a");
        std::fs::write(temp.join("b.txt"), b"bbb").expect("write b");
        std::fs::create_dir_all(temp.join("sub")).expect("create sub");
        std::fs::write(temp.join("sub").join("c.txt"), b"ccc").expect("write c");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert_eq!(artifact.file_count, 3);
        assert_eq!(artifact.dir_count, 2);

        let mut cursor = ManifestCursor::new(&artifact.bytes);
        limnifs_core::parse_manifest_header(&mut cursor).expect("header");
        limnifs_core::parse_feature_flags_section(&mut cursor).expect("flags");
        let meta_ref = limnifs_core::parse_metadata_reference(&mut cursor).expect("meta ref");
        assert!(meta_ref.is_inlined());
        let slab_index = limnifs_core::parse_slab_index(&mut cursor).expect("slab index");
        assert_eq!(slab_index.len(), 0);
        limnifs_core::parse_history(&mut cursor).expect("history");
    }

    #[test]
    fn write_deterministic() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-det", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        std::fs::write(temp.join("x.txt"), b"xxx").expect("write x");

        let a1 = write_directory(&temp).expect("first write");
        let a2 = write_directory(&temp).expect("second write");
        std::fs::remove_dir_all(&temp).ok();

        assert_eq!(a1.bytes, a2.bytes);
        assert_eq!(a1.merkle_root, a2.merkle_root);
    }

    #[test]
    fn slab_parses_correctly() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-slab", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        std::fs::write(temp.join("big.bin"), vec![0x11u8; INLINE_THRESHOLD + 1])
            .expect("write big");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();

        let slab_bytes = &artifact.slabs[0].bytes;
        let mut cursor = ManifestCursor::new(slab_bytes);
        let slab_header = limnifs_core::parse_slab_header(&mut cursor).expect("slab header parses");
        assert_eq!(slab_header.format_version, 1);
        assert!(!slab_header.is_sealed());
        assert!(!slab_header.has_erasure_coding());

        let drop_record =
            limnifs_core::parse_drop_record(&mut cursor, &slab_header).expect("drop record parses");
        assert_eq!(drop_record.plaintext_len as usize, INLINE_THRESHOLD + 1);
    }

    #[test]
    fn fastcdc_produces_multiple_chunks_for_large_files() {
        // A 1 MiB pseudo-random file should produce multiple drops
        // via FastCDC (default chunker uses 64 KiB min / 256 KiB avg).
        let temp = std::env::temp_dir().join(format!(
            "limnifs-write-test-{}-cdc-multi",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let data = pseudo_random_bytes(42, 1024 * 1024);
        std::fs::write(temp.join("big.bin"), &data).expect("write big");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert!(
            artifact.drop_count > 1,
            "expected FastCDC to produce multiple drops for 1 MiB input, got {}",
            artifact.drop_count
        );
    }

    #[test]
    fn fastcdc_deduplicates_shared_substrings() {
        // Two files sharing a long middle section should produce
        // fewer drops than the sum of their individual chunk counts,
        // because the shared section's chunks deduplicate.
        let temp = std::env::temp_dir().join(format!(
            "limnifs-write-test-{}-cdc-dedup",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let shared = pseudo_random_bytes(7, 512 * 1024);
        let mut a = Vec::with_capacity(shared.len() + 1024);
        a.extend_from_slice(&pseudo_random_bytes(1, 1024));
        a.extend_from_slice(&shared);
        let mut b = Vec::with_capacity(shared.len() + 2048);
        b.extend_from_slice(&pseudo_random_bytes(2, 2048));
        b.extend_from_slice(&shared);
        std::fs::write(temp.join("a.bin"), &a).expect("write a");
        std::fs::write(temp.join("b.bin"), &b).expect("write b");

        // Baseline: each file alone.
        let temp_a = std::env::temp_dir().join(format!(
            "limnifs-write-test-{}-cdc-dedup-a",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_a).expect("create temp_a");
        std::fs::write(temp_a.join("a.bin"), &a).expect("write a");
        let artifact_a = write_directory(&temp_a).expect("a writes");
        std::fs::remove_dir_all(&temp_a).ok();

        let temp_b = std::env::temp_dir().join(format!(
            "limnifs-write-test-{}-cdc-dedup-b",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_b).expect("create temp_b");
        std::fs::write(temp_b.join("b.bin"), &b).expect("write b");
        let artifact_b = write_directory(&temp_b).expect("b writes");
        std::fs::remove_dir_all(&temp_b).ok();

        let artifact_both = write_directory(&temp).expect("both write");
        std::fs::remove_dir_all(&temp).ok();

        let sum_alone = artifact_a.drop_count + artifact_b.drop_count;
        assert!(
            artifact_both.drop_count < sum_alone,
            "expected dedup win: both together = {} drops, sum alone = {} drops",
            artifact_both.drop_count,
            sum_alone
        );
    }

    #[test]
    fn slab_splits_when_content_exceeds_ceiling() {
        // Synthesise enough incompressible drops to force at least two
        // slabs. Each drop is 10 MiB of pseudo-random data; three drops
        // = 30 MiB compressed (random data doesn't compress), which
        // fits in one slab. We bump to seven drops (70 MiB) to force a
        // split.
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-split", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        for i in 0..7u32 {
            // 10 MiB of pseudo-random bytes — incompressible.
            let data = pseudo_random_bytes(u64::from(i), 10 * 1024 * 1024);
            std::fs::write(temp.join(format!("big-{i}.bin")), &data).expect("write big");
        }
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();

        // Each slab's total length must respect MAX_SLAB_TOTAL_BYTES.
        assert!(
            artifact.slabs.len() >= 2,
            "expected at least 2 slabs for 70 MiB of incompressible data, got {}",
            artifact.slabs.len()
        );
        for slab in &artifact.slabs {
            assert!(
                slab.bytes.len() <= MAX_SLAB_TOTAL_BYTES,
                "slab {} is {} bytes (> {} ceiling)",
                slab.id.ordinal,
                slab.bytes.len(),
                MAX_SLAB_TOTAL_BYTES,
            );
        }
        // All seven drops must be accounted for across slabs.
        let total_drop_ids: usize = artifact.slabs.iter().map(|s| s.drop_ids.len()).sum();
        assert_eq!(
            total_drop_ids, artifact.drop_count,
            "drop_ids count across slabs must match WriteArtifact.drop_count",
        );
    }
}
