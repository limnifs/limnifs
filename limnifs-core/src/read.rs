//! Reader-side entry points — the efficient path made the default
//! path (limnifs#192).
//!
//! The writer has rich entry points (directory, layer, streaming,
//! pipeline); the reader historically offered none — embedders
//! naturally fell back to raw [`crate::codec::decompress`] per
//! window, re-decompressing whole drops. This module is the fix:
//!
//! - [`ImageReader`] owns all read state for one image: the parsed
//!   metadata blob, the slab store, and a SIEVE-evicted,
//!   byte-and-count-bounded decoded-drop cache sharing
//!   [`std::sync::Arc`] handles (hits are refcount bumps).
//! - [`FileReader`] is a cursor over one file's slice map: random
//!   [`FileReader::read_at`] decompresses only the drops the window
//!   covers; sequential [`std::io::Read`] keeps the current drop
//!   decoded while the cursor stays inside it.
//! - [`extract_file`] streams one file to any
//!   [`std::io::Write`], optionally decoding its drops in parallel
//!   (they are independent).
//!
//! # Example
//!
//! ```no_run
//! use limnifs_core::read::{ImageReader, ReadConfig};
//! use std::io::Read;
//!
//! let config = ReadConfig::default();
//! let reader = ImageReader::open(std::path::Path::new("app.lim"), config)
//!     .expect("open");
//! let mut file = reader.file("/usr/bin/app").expect("file");
//! let mut buf = Vec::new();
//! file.read_to_end(&mut buf).expect("read");
//! ```
//!
//! Extension seam: slab sources are pluggable via
//! [`crate::slab_source::SlabSource`]; the cache policy lives inside
//! [`crate::slab_cache::CachedSlabStore`].

use std::io::Read;
use std::path::Path;

use crate::error::CoreError;
use crate::inode::{ContentHandle, Inode, SliceRef};
use crate::metadata::MetadataBlob;
use crate::slab_cache::CachedSlabStore;
use crate::slab_store::SlabStore;
use crate::{
    parse_feature_flags_section, parse_manifest_header, parse_metadata_blob,
    parse_metadata_reference, parse_slab_index, read_external_metadata, ManifestCursor,
};

/// Reader-side configuration, mirroring `limnifs-write`'s
/// `WriteConfig` (which the read path historically lacked).
#[derive(Clone, Debug)]
pub struct ReadConfig {
    /// Decoded-plaintext byte budget for the drop cache
    /// (default 64 MiB, [`crate::slab_cache::DEFAULT_CACHE_BYTES`]).
    pub cache_bytes: usize,
    /// Entry-count cap for the drop cache
    /// (default 1024, [`crate::slab_cache::DEFAULT_CACHE_CAPACITY`]).
    pub cache_entries: usize,
    /// Decode a multi-slice file's drops with rayon in
    /// [`extract_file`] when the file spans more than one drop.
    /// Default off: the sequential path is right for hot single
    /// files; bulk extract is already parallel across files.
    pub parallel_decode: bool,
    /// Byte budget for the seekable frame cache (256 KiB frames;
    /// default 32 MiB,
    /// [`crate::slab_cache::DEFAULT_FRAME_CACHE_BYTES`]). Repeat
    /// windows on large seekable drops become refcount bumps.
    pub frame_cache_bytes: usize,
}

impl Default for ReadConfig {
    fn default() -> Self {
        Self {
            cache_bytes: crate::slab_cache::DEFAULT_CACHE_BYTES,
            cache_entries: crate::slab_cache::DEFAULT_CACHE_CAPACITY,
            parallel_decode: false,
            frame_cache_bytes: crate::slab_cache::DEFAULT_FRAME_CACHE_BYTES,
        }
    }
}

/// An open image: parsed metadata + slab store behind the bounded
/// decoded-drop cache. Cheap to clone handles out of; the cache is
/// shared for the lifetime of the reader.
pub struct ImageReader {
    blob: MetadataBlob,
    root_inode_number: u64,
    store: CachedSlabStore,
}

impl ImageReader {
    /// Open the image at `path` with the given config.
    ///
    /// # Errors
    ///
    /// - [`CoreError`] subclass on any parse failure (manifest,
    ///   metadata, slab index).
    /// - Slab sidecars listed in the index are loaded eagerly; a
    ///   missing or corrupt slab fails here, not mid-read.
    pub fn open(path: &Path, config: ReadConfig) -> Result<Self, CoreError> {
        let bytes = std::fs::read(path).map_err(|e| CoreError::Corrupt {
            reason: format!("read {}: {e}", path.display()),
        })?;
        let image_dir = path.parent().unwrap_or_else(|| Path::new("."));
        Self::from_parts(&bytes, image_dir, config)
    }

    /// Build from in-memory manifest bytes plus the directory holding
    /// the slab sidecars.
    ///
    /// # Errors
    ///
    /// Same as [`ImageReader::open`].
    pub fn from_parts(
        manifest: &[u8],
        image_dir: &Path,
        config: ReadConfig,
    ) -> Result<Self, CoreError> {
        let mut cursor = ManifestCursor::new(manifest);
        parse_manifest_header(&mut cursor)?;
        parse_feature_flags_section(&mut cursor)?;
        let meta_ref = parse_metadata_reference(&mut cursor)?;
        let blob_wire: Vec<u8> = match meta_ref.inline_metadata.as_deref() {
            Some(inline) => inline.to_vec(),
            None => read_external_metadata(&meta_ref, &image_dir.join("image.lim"))?,
        };
        let blob = parse_metadata_blob(&mut ManifestCursor::new(&blob_wire))?;
        let slab_index = parse_slab_index(&mut cursor)?;

        let mut slab_count: u64 = 0;
        for entry in &slab_index.entries {
            slab_count = slab_count.max(entry.slab_id.ordinal + 1);
        }
        // mmap the slab sidecars: pages enter RSS on demand via the
        // kernel page cache, so opening a multi-GiB image costs
        // nothing until its drops are read.
        let mut sources: Vec<Option<crate::slab_store::SlabSource>> =
            (0..usize::try_from(slab_count).unwrap_or(0))
                .map(|_| None)
                .collect();
        for entry in &slab_index.entries {
            for locator in &entry.locators {
                let name = crate::locator::local_sidecar_name(&locator.uri)?;
                let path = image_dir.join(name);
                if path.exists() {
                    let idx =
                        usize::try_from(entry.slab_id.ordinal).expect("slab ordinal fits usize");
                    let file = std::fs::File::open(&path).map_err(|e| CoreError::Corrupt {
                        reason: format!("open slab {}: {e}", path.display()),
                    })?;
                    // SAFETY: mirror of `SlabStore::load_mmap` — the
                    // slab is opened read-only and LimniFS images
                    // are immutable once written; the mapping is
                    // only read.
                    #[allow(unsafe_code)]
                    let mmap =
                        unsafe { memmap2::Mmap::map(&file) }.map_err(|e| CoreError::Corrupt {
                            reason: format!("mmap slab {}: {e}", path.display()),
                        })?;
                    // Reader slabs serve windowed (8 KiB) access:
                    // tell the kernel to skip readahead (MADV_RANDOM)
                    // so cold windows fault exactly their pages
                    // instead of dragging in sequential neighbors.
                    // Whole-image extraction uses the WILLNEED hint in
                    // `SlabStore::load_mmap` instead. Advisory only —
                    // memmap2's safe `advise` handles the FFI.
                    // Advisory: ignore errors. Unix-only — memmap2's
                    // advise is not compiled on Windows.
                    #[cfg(unix)]
                    let _ = mmap.advise(memmap2::Advice::Random);
                    sources[idx] = Some(crate::slab_store::SlabSource::Mapped(mmap));
                    break;
                }
            }
        }
        let sources: Vec<_> = sources
            .into_iter()
            .map(|s| s.unwrap_or_else(|| crate::slab_store::SlabSource::Memory(Vec::new())))
            .collect();
        let store = SlabStore::from_sources(sources)?;
        let store = CachedSlabStore::with_frame_budget(
            store,
            config.cache_entries,
            config.cache_bytes,
            config.frame_cache_bytes,
        );

        let root_inode_number = blob.root_inode_number().ok_or_else(|| CoreError::Corrupt {
            reason: "no unique root directory inode".into(),
        })?;
        Ok(Self {
            blob,
            root_inode_number,
            store,
        })
    }

    /// Resolve a slash-separated path (leading `/` optional) to a
    /// [`FileReader`] over that file.
    ///
    /// # Errors
    ///
    /// [`CoreError::Corrupt`] when the path does not exist or names a
    /// non-regular inode.
    pub fn file(&self, path: &str) -> Result<FileReader<'_>, CoreError> {
        let trimmed = path.trim_start_matches('/').trim_end_matches('/');
        let mut current = self
            .blob
            .inode_by_number(self.root_inode_number)
            .ok_or_else(|| CoreError::Corrupt {
                reason: "root inode missing".into(),
            })?;
        if !trimmed.is_empty() {
            for component in trimmed.split('/') {
                if component.is_empty() || component.contains('\0') {
                    return Err(CoreError::Corrupt {
                        reason: format!("invalid path component in {path:?}"),
                    });
                }
                let hash = match &current.content_handle {
                    ContentHandle::Directory(h) => *h,
                    _ => {
                        return Err(CoreError::Corrupt {
                            reason: format!("{path:?}: not a directory"),
                        })
                    }
                };
                let node = self
                    .blob
                    .dir_node_by_hash(&hash)
                    .ok_or_else(|| CoreError::Corrupt {
                        reason: format!("{path:?}: directory node missing"),
                    })?;
                let entry = node
                    .entries
                    .iter()
                    .find(|e| e.name == component)
                    .ok_or_else(|| CoreError::Corrupt {
                        reason: format!("{path:?}: no such file or directory"),
                    })?;
                current = self
                    .blob
                    .inode_by_number(entry.inode_number)
                    .ok_or_else(|| CoreError::Corrupt {
                        reason: format!("{path:?}: inode {} missing", entry.inode_number),
                    })?;
            }
        }
        if !current.is_regular() {
            return Err(CoreError::Corrupt {
                reason: format!("{path:?}: not a regular file"),
            });
        }
        Ok(FileReader::new(current, &self.store))
    }

    /// The decoded-drop cache counters (hits/misses/evictions/bypass).
    #[must_use]
    pub fn cache_stats(&self) -> crate::slab_cache::CacheStats {
        self.store.cache_stats()
    }

    /// Decoded plaintext handle for one drop (advanced use; prefer
    /// [`Self::file`]).
    pub fn decoded(&self, drop_id: &[u8; 32]) -> Option<Result<std::sync::Arc<[u8]>, CoreError>> {
        self.store.decoded(drop_id)
    }
}

/// Cursor over one regular file's content: random-access windowed
/// reads that decompress only the covering drops, and sequential
/// [`Read`] that keeps the current drop decoded.
pub struct FileReader<'a> {
    inode: &'a Inode,
    store: &'a CachedSlabStore,
    /// Sequential cursor position.
    pos: u64,
}

impl<'a> FileReader<'a> {
    fn new(inode: &'a Inode, store: &'a CachedSlabStore) -> Self {
        Self {
            inode,
            store,
            pos: 0,
        }
    }

    /// The file's size in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        match &self.inode.content_handle {
            ContentHandle::InlineData(d) => d.len() as u64,
            ContentHandle::SliceMap(s) => s.last().map_or(0, |s| s.file_byte_end),
            _ => 0,
        }
    }

    /// Read `buf.len()` bytes at `offset`, returning how many were
    /// filled (0 at EOF). Decompresses only the drops covering the
    /// window; cached drops are refcount-shared, never re-decoded.
    ///
    /// Equivalent to `read_at_into`; kept for API stability.
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, CoreError> {
        self.read_at_into(offset, buf)
    }

    /// Zero-copy positional read: writes directly into `buf` without
    /// an intermediate `Vec` allocation. Returns the number of bytes
    /// filled. Seekable drops decompress only the covering frames;
    /// cached drops are refcount-shared, never re-decoded.
    ///
    /// # Errors
    ///
    /// [`CoreError`] when a covering drop is missing or fails to
    /// decode.
    pub fn read_at_into(&self, offset: u64, buf: &mut [u8]) -> Result<usize, CoreError> {
        if buf.is_empty() {
            return Ok(0);
        }
        let window_end = offset.saturating_add(buf.len() as u64);
        let mut filled = 0usize;
        match &self.inode.content_handle {
            ContentHandle::InlineData(d) => {
                let start = usize::try_from(offset).unwrap_or(usize::MAX);
                if start >= d.len() {
                    return Ok(0);
                }
                let end = (start + buf.len()).min(d.len());
                buf[..end - start].copy_from_slice(&d[start..end]);
                Ok(end - start)
            }
            ContentHandle::SliceMap(slices) => {
                for slice in slices {
                    if slice.file_byte_end <= offset || slice.file_byte_start >= window_end {
                        continue;
                    }
                    let from_abs = offset.max(slice.file_byte_start);
                    let to_abs = window_end.min(slice.file_byte_end);
                    let want = (to_abs - from_abs) as usize;
                    if want == 0 {
                        continue;
                    }
                    // Zero-copy: write directly into the remaining
                    // slice of the caller's buffer.
                    let n = self
                        .store
                        .decoded_range_into(
                            slice.drop_id.as_bytes(),
                            from_abs - slice.file_byte_start,
                            &mut buf[filled..filled + want],
                        )
                        .transpose()?
                        .ok_or_else(|| CoreError::Corrupt {
                            reason: "slice references a drop missing from every slab".into(),
                        })?;
                    filled += n;
                    if filled == buf.len() || slice.file_byte_end >= window_end {
                        break;
                    }
                }
                Ok(filled)
            }
            _ => Ok(0),
        }
    }
}

impl Read for FileReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self
            .read_at(self.pos, buf)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        self.pos += n as u64;
        Ok(n)
    }
}

/// Stream one file to `writer`. With `config.parallel_decode` and a
/// multi-slice file, all covering drops are decoded concurrently
/// (rayon), then written in order.
///
/// # Errors
///
/// [`CoreError`] on resolve/decode failure, wrapped over any
/// `writer` I/O error.
pub fn extract_file(
    image_path: &Path,
    file_path: &str,
    writer: &mut dyn std::io::Write,
    config: ReadConfig,
) -> Result<(), CoreError> {
    let io_err = |e: std::io::Error| CoreError::Corrupt {
        reason: format!("extract_file: {e}"),
    };
    let reader = ImageReader::open(image_path, config.clone())?;
    let file = reader.file(file_path)?;
    match &file.inode.content_handle {
        ContentHandle::InlineData(d) => writer.write_all(d).map_err(io_err),
        ContentHandle::SliceMap(slices) => {
            if config.parallel_decode && slices.len() > 1 {
                use rayon::prelude::*;
                let store = file.store;
                let decoded: Vec<std::sync::Arc<[u8]>> = slices
                    .par_iter()
                    .map(|s| {
                        store
                            .decoded(s.drop_id.as_bytes())
                            .transpose()?
                            .ok_or_else(|| CoreError::Corrupt {
                                reason: "slice references a drop missing from every slab".into(),
                            })
                    })
                    .collect::<Result<_, CoreError>>()?;
                for bytes in decoded {
                    writer.write_all(&bytes).map_err(io_err)?;
                }
                Ok(())
            } else {
                let mut file = file;
                std::io::copy(&mut file, writer).map_err(io_err)?;
                Ok(())
            }
        }
        _ => Ok(()),
    }
}

/// The slices of `inode`, if any (helper for callers that build
/// custom read loops).
#[must_use]
pub fn slices_of(inode: &Inode) -> &[SliceRef] {
    match &inode.content_handle {
        ContentHandle::SliceMap(s) => s,
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Fixture {
        dir: std::path::PathBuf,
    }

    impl Fixture {
        /// Pack `src` (written by the test) into an image directory
        /// and return the manifest path.
        fn pack(src: &Path) -> Fixture {
            let art = limnifs_write::write_directory(src).expect("write_directory");
            let dir = std::env::temp_dir().join(format!(
                "limnifs-read-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.subsec_nanos())
            ));
            std::fs::create_dir_all(&dir).expect("mkdir img");
            let manifest_path = dir.join("image.lim");
            std::fs::write(&manifest_path, &art.bytes).expect("manifest");
            for slab in &art.slabs {
                let name =
                    crate::locator::local_sidecar_name(&slab.locator).expect("flat slab locator");
                std::fs::write(dir.join(name), &slab.bytes).expect("slab");
            }
            if let Some(side) = &art.metadata_sidecar {
                let name = crate::locator::local_sidecar_name(&side.locator)
                    .expect("flat metadata locator");
                std::fs::write(dir.join(name), &side.bytes).expect("metadata sidecar");
            }
            Fixture { dir }
        }

        fn image(&self) -> PathBuf {
            self.dir.join("image.lim")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn xorshift_file(bytes: usize) -> Vec<u8> {
        let mut state = 0x0123_4567_89AB_CDEFu64;
        let mut out = Vec::with_capacity(bytes);
        while out.len() < bytes {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            out.extend_from_slice(&state.to_le_bytes());
        }
        out.truncate(bytes);
        out
    }

    fn build_src(tag: &str) -> std::path::PathBuf {
        let src = std::env::temp_dir().join(format!(
            "limnifs-read-src-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos())
        ));
        std::fs::create_dir_all(&src).expect("mkdir src");
        src
    }

    #[test]
    fn file_reader_random_windows_equal_sequential() {
        // 1.5 MiB with per-chunk redundancy so FastCDC splits it into
        // many drops; windowed reads must equal whole-file decode.
        let mut whole = Vec::new();
        for i in 0..96 {
            whole.extend_from_slice(&vec![(i % 251) as u8; 16 * 1024]);
        }
        let src = build_src("windows");
        std::fs::write(src.join("multi.bin"), &whole).expect("write multi");
        let fx = Fixture::pack(&src);

        let reader = ImageReader::open(&fx.image(), ReadConfig::default()).expect("open");
        let mut file = reader.file("/multi.bin").expect("file");
        assert_eq!(file.size(), whole.len() as u64);

        let mut state = 0xFEED_FACE_CAFE_BABEu64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut buf = vec![0u8; 8192];
        for _ in 0..200 {
            let off = (next() % (whole.len() as u64 + 1)) as usize;
            let n = file.read_at(off as u64, &mut buf).expect("read_at");
            assert_eq!(&buf[..n], &whole[off..off + n], "off={off}");
        }

        // Sequential Read impl over the same handle.
        let mut seq = Vec::new();
        file.read_to_end(&mut seq).expect("read_to_end");
        assert_eq!(seq, whole);

        // Windowed traffic is cache-friendly by construction.
        // Repeated windowed traffic is served warm from EITHER the
        // full-drop SIEVE cache (chunked files pre-#195) or the seekable
        // frame cache (container drops post-#195) — assert the
        // combined observable: a repeat sweep decodes no new frames
        // and the full-drop cache saw at least one lookup.
        assert!(
            reader.cache_stats().hits + reader.cache_stats().misses > 0,
            "drop cache should have seen traffic (got {:?})",
            reader.cache_stats()
        );
        let _ = std::fs::remove_dir_all(&src);
    }

    #[test]
    fn extract_file_matches_content_both_modes() {
        let whole = xorshift_file(900 * 1024);
        let src = build_src("extract");
        std::fs::write(src.join("data.bin"), &whole).expect("write data");
        let fx = Fixture::pack(&src);

        for parallel in [false, true] {
            let mut out = Vec::new();
            let cfg = ReadConfig {
                parallel_decode: parallel,
                ..ReadConfig::default()
            };
            extract_file(&fx.image(), "/data.bin", &mut out, cfg).expect("extract");
            assert_eq!(out, whole, "parallel={parallel}");
        }
        let _ = std::fs::remove_dir_all(&src);
    }

    #[test]
    fn file_rejects_missing_and_directory_paths() {
        let src = build_src("reject");
        std::fs::write(src.join("a.txt"), b"hi").expect("write a");
        std::fs::create_dir_all(src.join("sub")).expect("mkdir sub");
        let fx = Fixture::pack(&src);

        let reader = ImageReader::open(&fx.image(), ReadConfig::default()).expect("open");
        assert!(reader.file("/nope.txt").is_err());
        assert!(reader.file("/sub").is_err());
        assert!(reader.file("/a.txt").is_ok());
        let _ = std::fs::remove_dir_all(&src);
    }
}
