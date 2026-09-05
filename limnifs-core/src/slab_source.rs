//! Slab source trait — abstraction over how slab bytes are accessed.
//!
//! Today only [`MmapSlabSource`] is wired (sync, mmap-backed). The
//! trait exists so a future Linux `IoUringSlabSource` (batched
//! submission queues) can slot in behind the same interface without
//! touching callers.
//!
//! The trait is intentionally NOT `async` to keep the dependency
//! graph clean (no `tokio` / `async-trait`). When the io_uring impl
//! lands it can present a sync surface backed by `io_uring_submit`
//! + `io_uring_wait` — that's still synchronous from the caller's
//! view, just batched internally.
//!
//! See `TODO.impl/03-core-reader/03-async-slab-source.md`.

use crate::error::CoreError;
use crate::slab_store::SlabStore;

/// Behaviour every slab source implements.
///
/// `Send + Sync` so the source can be shared across rayon workers
/// (parallel extract, parallel `cat-multi`).
pub trait SlabSource: Send + Sync {
    /// Fetch the plaintext of `drop_id` into an owned `Vec<u8>`.
    ///
    /// Returns:
    /// - `None` if no slab contains this drop.
    /// - `Some(Err(..))` if the slab is corrupt or the codec is unsupported.
    /// - `Some(Ok(bytes))` on success.
    #[must_use]
    fn plaintext_for(&self, drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, CoreError>>;

    /// Number of slabs in the source.
    #[must_use]
    fn slab_count(&self) -> usize;

    /// Number of unique drops indexed across all slabs.
    #[must_use]
    fn drop_count(&self) -> usize;
}

/// Synchronous mmap-backed slab source. Wraps a [`SlabStore`] and
/// delegates every method. Today this is the only production impl;
/// future Linux builds can add `IoUringSlabSource` behind a feature
/// flag and callers don't change.
pub struct MmapSlabSource {
    inner: SlabStore,
}

impl MmapSlabSource {
    /// Wrap an existing SlabStore. The store typically comes from
    /// `SlabStore::load_mmap(manifest_path, slab_index)`.
    #[must_use]
    pub fn new(inner: SlabStore) -> Self {
        Self { inner }
    }

    /// Borrow the underlying SlabStore for callers that need its
    /// richer API (e.g. `set_dictionaries`).
    #[must_use]
    pub fn inner(&self) -> &SlabStore {
        &self.inner
    }

    /// Mutably borrow the underlying SlabStore (e.g. to call
    /// `set_dictionaries`).
    pub fn inner_mut(&mut self) -> &mut SlabStore {
        &mut self.inner
    }
}

/// A [`SlabSource`] that consults a chain of stores in order —
/// the layer's local slabs first, then each base. This is the read
/// side of `write_layer`: drops the layer only references resolve
/// from the base images' slabs.
pub struct ChainedSlabSource<'a> {
    chain: Vec<&'a dyn SlabSource>,
}

impl<'a> ChainedSlabSource<'a> {
    /// Build from local-first stores. An empty chain answers `None`
    /// for everything.
    #[must_use]
    pub fn new(chain: Vec<&'a dyn SlabSource>) -> Self {
        Self { chain }
    }
}

impl SlabSource for ChainedSlabSource<'_> {
    fn plaintext_for(&self, drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, CoreError>> {
        self.chain.iter().find_map(|s| s.plaintext_for(drop_id))
    }
    fn slab_count(&self) -> usize {
        self.chain.iter().map(|s| s.slab_count()).sum()
    }
    fn drop_count(&self) -> usize {
        self.chain.iter().map(|s| s.drop_count()).sum()
    }
}

impl SlabSource for MmapSlabSource {
    fn plaintext_for(&self, drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, CoreError>> {
        self.inner.plaintext_for(drop_id)
    }

    fn slab_count(&self) -> usize {
        self.inner.slab_count()
    }

    fn drop_count(&self) -> usize {
        self.inner.drop_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: MmapSlabSource delegates correctly to a fresh
    /// SlabStore. The full SlabStore round-trip is exercised in
    /// slab_store.rs and slab_cache.rs.
    #[test]
    fn mmap_source_delegates_to_inner() {
        let store = SlabStore::default();
        let source = MmapSlabSource::new(store);
        assert_eq!(source.slab_count(), 0);
        assert_eq!(source.drop_count(), 0);
        // No slabs → no drops resolvable.
        let drop_id = [0u8; 32];
        assert!(source.plaintext_for(&drop_id).is_none());
    }
}
