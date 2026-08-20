//! io_uring slab source — Linux-only batched submission queue.
//!
//! **Status:** STUB. Compiles only on Linux behind the
//! `io-uring` feature flag. Unfinished until Linux CI validates
//! the `io-uring` crate integration.
//!
//! See `TODO.impl/03-core-reader/03-async-slab-source.md`.

#![cfg(all(target_os = "linux", feature = "io-uring"))]

use crate::error::CoreError;
use crate::slab_source::SlabSource;

/// Linux io_uring-backed slab source. Batches drop lookups via
/// submission queues.
///
/// **NOT IMPLEMENTED** — body panics.
/// Task: `TODO.impl/03-core-reader/03-async-slab-source.md`.
pub struct IoUringSlabSource {}

impl IoUringSlabSource {
    /// Construct from a manifest path + slab index.
    #[must_use]
    pub fn new(
        _manifest_path: &std::path::Path,
        _slab_index: &crate::slab_index::SlabIndex,
    ) -> Self {
        todo!("IoUringSlabSource requires io-uring + Linux CI") // TODO.impl/03-core-reader/03-async-slab-source.md
    }
}

impl SlabSource for IoUringSlabSource {
    fn plaintext_for(&self, _drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, CoreError>> {
        todo!("IoUringSlabSource::plaintext_for needs io_uring") // TODO.impl/03-core-reader/03-async-slab-source.md
    }

    fn slab_count(&self) -> usize {
        todo!() // TODO.impl/03-core-reader/03-async-slab-source.md
    }

    fn drop_count(&self) -> usize {
        todo!() // TODO.impl/03-core-reader/03-async-slab-source.md
    }
}
