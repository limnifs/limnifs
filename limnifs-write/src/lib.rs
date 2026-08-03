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
pub mod file_categorizer;
pub mod flatten;
pub mod rw;
pub mod turnover;

pub use config::{
    profile, CategorizerConfig, ChunkingConfig, CodecRegistry, CodecTunables, Defaults,
    DictionaryConfig, EncryptionConfig, TournamentConfig, WriteConfig,
};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::chunker::FastCDC;
use limnifs_core::{
    compute_merkle_root, hash_empty_section, hash_section, ManifestHeader, SectionHashes,
    FEATURE_FLAGS_SECTION_VERSION, HISTORY_SECTION_VERSION, INODE_FLAG_INLINE_DATA,
    INODE_FLAG_SHARED_INLINE, METADATA_REFERENCE_SECTION_VERSION_2, SLAB_INDEX_SECTION_VERSION,
};
use limnifs_format::{ManifestRoot, SlabId};

/// Inline-data threshold: files at or below this size get inline data
/// in their inode. Larger files are stored as drops in a slab.
pub const INLINE_THRESHOLD: usize = 4096;

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

/// Create an image with a custom [`WriteConfig`] (e.g. from a profile).
pub fn write_directory_with_config(
    root: &Path,
    config: &WriteConfig,
) -> Result<WriteArtifact, WriteError> {
    use rayon::prelude::*;

    let mut ctx = WriteContext::new();

    let root_inode_number = ctx.walk(root)?;
    ctx.root_inode_number = root_inode_number;

    let pending = std::mem::take(&mut ctx.pending_files);
    if !pending.is_empty() {
        let chunker = ctx.chunker.clone();
        let classifier = ctx.classifier;
        let text_codec = config.text_codec_id().unwrap_or(0x04);
        let binary_codec = config.binary_codec_id().unwrap_or(0x01);
        let brotli_quality = config.codec_tunables.brotli.quality;
        let results: Vec<ChunkedFileResult> = pending
            .par_iter()
            .map(|pf| {
                process_file(
                    pf,
                    &chunker,
                    classifier,
                    text_codec,
                    binary_codec,
                    brotli_quality,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        for (pf, result) in pending.iter().zip(results) {
            ctx.merge_chunked_file(pf, result);
        }
    }

    let artifact = ctx.assemble();
    Ok(artifact)
}

/// One chunk of a file before dedup: (`drop_id`, `plaintext`, `compressed`, `codec`).
type RawDrop = ([u8; 32], Vec<u8>, Vec<u8>, u8);
/// Result of parallel file processing: the drop data (uncompressed,
struct ChunkedFileResult {
    drops: Vec<RawDrop>, // (id, plaintext, compressed, codec)
    slices: Vec<PendingSlice>,
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
    _pf: &PendingFile,
    data: &[u8],
    cat: file_categorizer::Categorization,
    brotli_quality: u8,
) -> Result<ChunkedFileResult, WriteError> {
    let drop_id = hash_section(data);

    let brotli_c = limnifs_core::codec::compress_with_options(
        limnifs_core::codec::CODEC_BROTLI,
        data,
        brotli_quality,
    )
    .map_err(|e| WriteError::Io(std::io::Error::other(format!("brotli compress: {e}"))))?;

    let zstd_c =
        limnifs_core::codec::compress(limnifs_core::codec::CODEC_ZSTD, data).unwrap_or_default();

    let (mut best_codec, mut best_compressed) = if brotli_c.len() <= zstd_c.len() {
        (limnifs_core::codec::CODEC_BROTLI, brotli_c)
    } else {
        (limnifs_core::codec::CODEC_ZSTD, zstd_c)
    };

    // Only try the specialized codec if the general-purpose ratio
    // is poor (>15%) — otherwise the specialized codec is unlikely
    // to help and may be very slow (FLAC, FSST).
    let general_ratio = best_compressed.len() as f64 / data.len() as f64;
    if general_ratio > 0.15 || cat.codec_id == limnifs_core::codec::CODEC_RICEPP {
        if let Ok(spec_c) = limnifs_core::codec::compress(cat.codec_id, data) {
            if spec_c.len() < best_compressed.len() {
                best_codec = cat.codec_id;
                best_compressed = spec_c;
            }
        }
    }

    let file_len = u64::try_from(data.len()).unwrap_or(u64::MAX);
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
/// FSST+Brotli for CSV), the whole file is compressed as a single
/// drop with the categorizer's chosen codec + parameters. Otherwise
/// falls through to `FastCDC` + per-chunk classify.
fn process_file(
    pf: &PendingFile,
    chunker: &FastCDC,
    classifier: classifier::Classifier,
    text_codec: u8,
    binary_codec: u8,
    brotli_quality: u8,
) -> Result<ChunkedFileResult, WriteError> {
    let data = std::fs::read(&pf.path)?;
    let file_len = data.len();

    // File-level categorizer path: skip FastCDC if a specialized
    // codec claims this file. Codecs that require container headers
    // (FLAC, Rice++) always use the whole-file path. Byte-oriented
    // codecs (FSST+Brotli, Brotli, ZSTD) use FastCDC for files above
    // WHOLE_FILE_MAX_SIZE to parallelize across rayon workers.
    if let Some(cat) = file_categorizer::default_registry().categorize(&pf.path, &data) {
        let needs_whole_file = matches!(
            cat.codec_id,
            limnifs_core::codec::CODEC_FLAC | limnifs_core::codec::CODEC_RICEPP
        );
        if needs_whole_file || file_len <= WHOLE_FILE_MAX_SIZE {
            return process_whole_file_drop(pf, &data, cat, brotli_quality);
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
    use rayon::prelude::*;
    let drops: Vec<([u8; 32], Vec<u8>, Vec<u8>, u8)> = unique_chunks
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
            let (codec_id, compressed) = if preferred_codec == limnifs_core::codec::CODEC_STORE {
                (limnifs_core::codec::CODEC_STORE, chunk.to_vec())
            } else {
                match limnifs_core::codec::compress_with_options(
                    preferred_codec,
                    chunk,
                    brotli_quality,
                ) {
                    Ok(c) if c.len() < chunk.len() => (preferred_codec, c),
                    _ => (limnifs_core::codec::CODEC_STORE, chunk.to_vec()),
                }
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
    compressed: Vec<u8>,
    codec: u8,
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
    /// Maps BLAKE3 hash of inline data → index into `shared_inline_table`.
    /// Populated by `build_shared_inline_table` in `assemble`.
    shared_inline_map: HashMap<[u8; 32], usize>,
    /// Unique inline data entries that appear more than once.
    /// Stored at the end of the metadata blob for dedup.
    shared_inline_table: Vec<Vec<u8>>,
}

impl WriteContext {
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
                self.drops.push(PendingDrop {
                    id: drop_id,
                    plaintext_len: u32::try_from(plaintext.len()).unwrap_or(u32::MAX),
                    compressed,
                    codec,
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
        let (codec, compressed) = match class {
            classifier::Class::Text | classifier::Class::Code | classifier::Class::Binary => {
                let c = limnifs_core::codec::compress_lz4_with_size(plaintext);
                (limnifs_core::codec::CODEC_LZ4, c)
            }
            _ => (limnifs_core::codec::CODEC_STORE, plaintext.to_vec()),
        };
        PendingDrop {
            id: drop_id,
            plaintext_len: u32::try_from(plaintext.len()).unwrap_or(u32::MAX),
            compressed,
            codec,
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
        let metadata_codec = limnifs_core::codec::best_compressible_codec();
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

        let hashes = SectionHashes {
            metadata: metadata_hash,
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
    if drops.is_empty() {
        return Vec::new();
    }

    let max_content = MAX_SLAB_TOTAL_BYTES.saturating_sub(SLAB_HEADER_LEN);
    let mut slabs: Vec<SlabArtifact> = Vec::new();
    let mut current: Vec<&PendingDrop> = Vec::new();
    let mut current_size: usize = 0;

    for drop in drops {
        let footprint = drop.slab_footprint();
        if !current.is_empty() && current_size + footprint > max_content {
            let ordinal = u64::try_from(slabs.len()).expect("slab count fits u64");
            slabs.push(encode_slab(ordinal, &current));
            current.clear();
            current_size = 0;
        }
        current.push(drop);
        current_size += footprint;
    }
    if !current.is_empty() {
        let ordinal = u64::try_from(slabs.len()).expect("slab count fits u64");
        slabs.push(encode_slab(ordinal, &current));
    }
    slabs
}

/// Encode a single slab from a non-empty slice of drops. Per-slab
/// `offset_in_window` is computed fresh; there is no global offset
/// state on `PendingDrop`.
fn encode_slab(ordinal: u64, drops: &[&PendingDrop]) -> SlabArtifact {
    let mut drop_records = Vec::new();
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
        drop_records.push(limnifs_core::drop_record::NO_DICT); // dict_id: no dictionary
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
