//! Slab store — owns all slab bytes for an image and provides O(1)
//! `DropId` → plaintext lookup across slabs.
//!
//! Supports two storage modes:
//! - [`SlabSource::Memory`] — slab bytes loaded into a `Vec<u8>`.
//!   Used by [`SlabStore::load`] (eager read) and [`SlabStore::from_bytes`].
//! - [`SlabSource::Mapped`] — slab bytes memory-mapped via `memmap2`.
//!   Used by [`SlabStore::load_mmap`]. The kernel handles paging; only
//!   accessed pages enter RAM. Ideal for large images and random-access
//!   workloads.
//!
//! Streaming: [`SlabStore::stream_drop`] writes a drop's decompressed
//! plaintext directly to a [`std::io::Write`] impl, avoiding the
//! intermediate `Vec<u8>` allocation. For extracting a 1 GiB image,
//! this keeps peak RSS at `max_single_drop_size` instead of
//! `total_image_plaintext`.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use crate::error::CoreError;
use crate::slab_reader::{parse_slab, SlabView};

/// Storage mode for a single slab's bytes.
#[derive(Debug)]
pub enum SlabSource {
    /// Slab bytes held in an owned `Vec<u8>`.
    Memory(Vec<u8>),
    /// Slab bytes memory-mapped from a file. The kernel pages pages
    /// on demand; unaccessed regions consume no RSS.
    Mapped(memmap2::Mmap),
}

impl SlabSource {
    /// Read-only access to the slab bytes, regardless of storage mode.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Memory(v) => v.as_slice(),
            Self::Mapped(m) => m.as_ref(),
        }
    }
}

/// All slabs for one image, with a `DropId → slab_ordinal` index for
/// O(1) lookup. Slab ordinals match the order of the manifest's
/// `slab_index` entries.
#[derive(Debug, Default)]
pub struct SlabStore {
    /// One slab source per ordinal. Index = ordinal.
    slabs: Vec<SlabSource>,
    /// `DropId` → slab ordinal. Built once at load time.
    drop_index: HashMap<[u8; 32], usize>,
    /// `dict_id` → raw dictionary bytes. Populated by
    /// [`SlabStore::set_dictionaries`] when the caller has parsed
    /// the manifest's `dictionary_section`. Drops whose
    /// `DropRecord::dict_id != NO_DICT` consult this map at
    /// decompression time.
    dictionaries: HashMap<u8, Vec<u8>>,
}

impl SlabStore {
    /// Load every slab into memory (eager read). After this call,
    /// every drop referenced by the metadata blob is locatable via
    /// [`SlabStore::plaintext_for`].
    ///
    /// For large images, prefer [`SlabStore::load_mmap`] which avoids
    /// materialising all slab bytes in RSS.
    ///
    /// # Errors
    /// - [`CoreError::Corrupt`] if the slab index is empty, if any
    ///   slab file cannot be read, or if any slab fails to parse.
    pub fn load(
        manifest_path: &Path,
        slab_index: &crate::slab_index::SlabIndex,
    ) -> Result<Self, CoreError> {
        if slab_index.is_empty() {
            return Ok(Self::default());
        }

        let parent = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let mut slabs = Vec::with_capacity(slab_index.len());
        let mut drop_index: HashMap<[u8; 32], usize> = HashMap::new();

        for (ordinal, entry) in slab_index.entries.iter().enumerate() {
            let locator = entry
                .locators
                .first()
                .ok_or_else(|| CoreError::Corrupt {
                    reason: format!(
                        "slab_index entry {ordinal}: slab_id (ordinal {}) declares zero locators (unreachable)",
                        entry.slab_id.ordinal
                    ),
                })?;
            let slab_name = crate::locator::local_sidecar_name(&locator.uri).map_err(|e| {
                CoreError::Corrupt {
                    reason: format!("slab_index entry {ordinal}: {e}"),
                }
            })?;
            let slab_path = parent.join(slab_name);
            let bytes = std::fs::read(&slab_path).map_err(|e| CoreError::Corrupt {
                reason: format!(
                    "slab_index entry {ordinal}: cannot read slab file {}: {e}",
                    slab_path.display()
                ),
            })?;

            let view = parse_slab(&bytes)?;
            for record in view.drop_records() {
                drop_index.insert(*record.drop_id.as_bytes(), ordinal);
            }
            slabs.push(SlabSource::Memory(bytes));
        }

        Ok(Self {
            slabs,
            drop_index,
            dictionaries: HashMap::new(),
        })
    }

    /// Memory-map every slab file. The kernel pages data on demand;
    /// unaccessed slab regions consume no RSS. Ideal for large images
    /// and random-access workloads (locate, `read_random`, partial extract).
    ///
    /// # Errors
    /// - [`CoreError::Corrupt`] if any slab file cannot be opened,
    ///   mmap'd, or parsed.
    pub fn load_mmap(
        manifest_path: &Path,
        slab_index: &crate::slab_index::SlabIndex,
    ) -> Result<Self, CoreError> {
        if slab_index.is_empty() {
            return Ok(Self::default());
        }

        let parent = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let mut slabs = Vec::with_capacity(slab_index.len());
        let mut drop_index: HashMap<[u8; 32], usize> = HashMap::new();

        for (ordinal, entry) in slab_index.entries.iter().enumerate() {
            let locator = entry.locators.first().ok_or_else(|| CoreError::Corrupt {
                reason: format!("slab_index entry {ordinal}: zero locators (unreachable)"),
            })?;
            let slab_name = crate::locator::local_sidecar_name(&locator.uri).map_err(|e| {
                CoreError::Corrupt {
                    reason: format!("slab_index entry {ordinal}: {e}"),
                }
            })?;
            let slab_path = parent.join(slab_name);

            let file = std::fs::File::open(&slab_path).map_err(|e| CoreError::Corrupt {
                reason: format!(
                    "slab_index entry {ordinal}: cannot open slab file {}: {e}",
                    slab_path.display()
                ),
            })?;

            // SAFETY: mmap is safe here because:
            // 1. The file is opened read-only — no external mutation
            //    can corrupt the mapped bytes.
            // 2. LimniFS slab files are immutable after write — the
            //    writer never modifies a slab once sealed.
            // 3. We map the entire file; no sub-range calculation
            //    that could be wrong.
            // 4. The mapped bytes are only read (no mutation through
            //    the mapping).
            #[allow(unsafe_code)]
            let mmap = unsafe { memmap2::Mmap::map(&file) }.map_err(|e| CoreError::Corrupt {
                reason: format!(
                    "slab_index entry {ordinal}: mmap failed on {}: {e}",
                    slab_path.display()
                ),
            })?;

            // Hint the kernel to prefetch the slab pages. On first
            // access the page cache is cold; without this hint, each
            // page faults individually. MADV_WILLNEED triggers
            // readahead so pages are resident by the time we access
            // them. POSIX only — windows has no madvise; the hint is
            // advisory, never load-bearing.
            //
            // SAFETY: mmap is a valid read-only mapping of the slab
            // file. madvise with MADV_WILLNEED is a hint, not a
            // mutation; it cannot corrupt the mapping. The pointer
            // and length are derived from the Mmap which is valid.
            #[cfg(unix)]
            #[allow(unsafe_code)]
            {
                let ptr = mmap.as_ref().as_ptr() as *mut libc::c_void;
                let len = mmap.as_ref().len();
                // SAFETY: ptr and len describe the valid read-only
                // mmap region above. madvise with MADV_WILLNEED is a
                // prefetch hint — no mutation, no UB.
                unsafe {
                    let _ = libc::madvise(ptr, len, libc::MADV_WILLNEED);
                }
            }

            let view = parse_slab(&mmap[..])?;
            for record in view.drop_records() {
                drop_index.insert(*record.drop_id.as_bytes(), ordinal);
            }
            slabs.push(SlabSource::Mapped(mmap));
        }

        Ok(Self {
            slabs,
            drop_index,
            dictionaries: HashMap::new(),
        })
    }

    /// Build a store directly from in-memory slab bytes.
    ///
    /// # Errors
    /// - [`CoreError::Corrupt`] if any slab fails to parse.
    pub fn from_bytes(slabs: Vec<Vec<u8>>) -> Result<Self, CoreError> {
        let mut drop_index = HashMap::new();
        for (ordinal, bytes) in slabs.iter().enumerate() {
            let view = parse_slab(bytes)?;
            for record in view.drop_records() {
                drop_index.insert(*record.drop_id.as_bytes(), ordinal);
            }
        }
        Ok(Self {
            slabs: slabs.into_iter().map(SlabSource::Memory).collect(),
            drop_index,
            dictionaries: HashMap::new(),
        })
    }

    /// Number of slabs in the store.
    #[must_use]
    pub fn slab_count(&self) -> usize {
        self.slabs.len()
    }

    /// Number of unique drops indexed across all slabs.
    #[must_use]
    pub fn drop_count(&self) -> usize {
        self.drop_index.len()
    }

    /// Returns true if `drop_id` is present in any slab.
    #[must_use]
    pub fn contains(&self, drop_id: &[u8; 32]) -> bool {
        self.drop_index.contains_key(drop_id)
    }

    /// Iterator over every `DropId` known to this store. Used by
    /// `limnifs_write::write_layer` to build the base-image drop set
    /// so the layer can reference rather than re-encode matching chunks.
    #[must_use]
    pub fn drop_index_keys(&self) -> impl Iterator<Item = &[u8; 32]> {
        self.drop_index.keys()
    }

    /// Borrowed view onto a slab's bytes, regardless of storage mode.
    /// Index = slab ordinal.
    #[must_use]
    pub fn slab(&self, ordinal: usize) -> Option<&[u8]> {
        self.slabs.get(ordinal).map(SlabSource::as_bytes)
    }

    /// Fetch the plaintext of `drop_id` into an owned `Vec<u8>`.
    ///
    /// For streaming (no intermediate allocation), use
    /// [`SlabStore::stream_drop`] instead.
    ///
    /// Returns:
    /// - `None` if no slab contains this drop.
    /// - `Some(Err(..))` if the slab is corrupt or the codec is unsupported.
    /// - `Some(Ok(bytes))` on success.
    #[must_use]
    pub fn plaintext_for(&self, drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, CoreError>> {
        let ordinal = *self.drop_index.get(drop_id)?;
        let bytes = self.slabs.get(ordinal)?.as_bytes();
        let view: SlabView<'_> = parse_slab(bytes).ok()?;
        // Hand the SlabView a closure that looks up dict_id in our
        // dictionary map. Clones the dict bytes (cheap relative to
        // decompression). If `dictionaries` is empty, drops with
        // `dict_id == NO_DICT` are unaffected.
        view.plaintext_for_with_dict_lookup(drop_id, &|id| self.dictionaries.get(&id).cloned())
    }

    /// Set the dictionary table parsed from the manifest's
    /// `dictionary_section`. Drops whose `DropRecord::dict_id`
    /// references an id in this map will be decompressed via the
    /// dict-aware ZSTD path; drops with `dict_id == NO_DICT` (0xFF)
    /// are unaffected.
    ///
    /// Keys are `dict_id` (0..=254); values are raw dictionary bytes.
    pub fn set_dictionaries(&mut self, dictionaries: HashMap<u8, Vec<u8>>) {
        self.dictionaries = dictionaries;
    }

    /// Number of registered dictionaries.
    #[must_use]
    pub fn dictionary_count(&self) -> usize {
        self.dictionaries.len()
    }

    /// Stream a drop's decompressed plaintext directly to `writer`.
    /// Avoids the intermediate `Vec<u8>` allocation that
    /// [`plaintext_for`][Self::plaintext_for] creates.
    ///
    /// For extracting a multi-drop file, call this once per slice.
    /// Peak RSS stays at `max_single_drop_size`, not
    /// `total_file_size`.
    ///
    /// # Errors
    /// - [`CoreError::Corrupt`] if the drop is not found or
    ///   decompression fails.
    /// - [`CoreError::Io`] propagated from `writer`.
    pub fn stream_drop<W: Write>(
        &self,
        drop_id: &[u8; 32],
        writer: &mut W,
    ) -> Result<u64, CoreError> {
        let ordinal = *self
            .drop_index
            .get(drop_id)
            .ok_or_else(|| CoreError::Corrupt {
                reason: format!("stream_drop: drop {:02x?} not in any slab", &drop_id[..4]),
            })?;
        let bytes = self.slabs.get(ordinal).ok_or_else(|| CoreError::Corrupt {
            reason: format!("stream_drop: slab ordinal {ordinal} out of range"),
        })?;
        let view: SlabView<'_> = parse_slab(bytes.as_bytes())?;
        let plaintext = view
            .plaintext_for(drop_id)
            .ok_or_else(|| CoreError::Corrupt {
                reason: "stream_drop: slab view returned None for indexed drop".into(),
            })??;
        let len = plaintext.len() as u64;
        writer
            .write_all(&plaintext)
            .map_err(|e| CoreError::Corrupt {
                reason: format!("stream_drop: write failed: {e}"),
            })?;
        Ok(len)
    }
}

impl crate::slab_source::SlabSource for SlabStore {
    fn plaintext_for(&self, drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, CoreError>> {
        SlabStore::plaintext_for(self, drop_id)
    }
    fn slab_count(&self) -> usize {
        self.slab_count()
    }
    fn drop_count(&self) -> usize {
        self.drop_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_store_has_no_drops() {
        let store = SlabStore::default();
        assert_eq!(store.slab_count(), 0);
        assert_eq!(store.drop_count(), 0);
        let id = [0u8; 32];
        assert!(!store.contains(&id));
        assert!(store.plaintext_for(&id).is_none());
    }

    #[test]
    fn from_bytes_rejects_invalid_slab() {
        let result = SlabStore::from_bytes(vec![vec![0u8; 16]]);
        assert!(result.is_err(), "garbage slab must fail validation");
    }

    #[test]
    fn stream_drop_missing_returns_error() {
        let store = SlabStore::default();
        let mut output = Vec::new();
        let result = store.stream_drop(&[0u8; 32], &mut output);
        assert!(result.is_err());
        assert!(output.is_empty());
    }
}
