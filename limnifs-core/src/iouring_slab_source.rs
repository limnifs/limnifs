//! io_uring slab source — Linux-only batched submission queue.
//!
//! **Status:** STUB. Compiles only on Linux behind the
//! `io-uring` feature flag. The body is unfinished until a Linux
//! CI environment validates the `io-uring` crate integration.
//!
//! ## Why this exists
//!
//! On Linux 5.1+, `io_uring` provides batched async I/O via
//! submission/completion queues. For random-access slab reads
//! (mount, `cat-multi`, turnover on hot images), batching N drop
//! lookups into one `io_uring_submit` + `io_uring_wait` reduces
//! syscall overhead from N syscalls to 1.
//!
//! ## Why it's a stub today
//!
//! The macOS dev box doesn't have io_uring. The `io-uring` crate
//! is Linux-only. Writing the full impl requires Linux CI to
//! validate. This module:
//!
//! 1. Documents the planned API.
//! 2. Defines the struct + impl signature so downstream code can
//!    `#[cfg(all(target_os = "linux", feature = "io-uring"))]`
//!    reference it.
//! 3. The actual `plaintext_for` panics until implemented.
//!
//! When Linux CI lands:
//! 1. Add `io-uring = "0.6"` as a Linux-only dep.
//! 2. Implement `plaintext_for` via batched `io_uring_readv`.
//! 3. Run the differential test suite against `MmapSlabSource`.
//!
//! See `TODO.impl/03-core-reader/03-async-slab-source.md`.

#![cfg(all(target_os = "linux", feature = "io-uring"))]

use crate::error::CoreError;
use crate::slab_source::SlabSource;

/// Linux io_uring-backed slab source. Batches drop lookups via
/// submission queues.
///
/// **NOT IMPLEMENTED** — body panics. See module docs.
/// Task: `TODO.impl/03-core-reader/03-async-slab-source.md`.
pub struct IoUringSlabSource {
    // Planned fields:
    // ring: io_uring::IoUring,
    // slab_files: Vec<std::fs::File>,
    // drop_index: HashMap<[u8; 32], (usize, u64, u32)>, // (ordinal, offset, len)
}

impl IoUringSlabSource {
    /// Construct from a manifest path + slab index. Opens all slab
    /// files and sets up the io_uring instance.
    #[must_use]
    pub fn new(
        _manifest_path: &std::path::Path,
        _slab_index: &crate::slab_index::SlabIndex,
    ) -> Self {
        // TODO.impl/03-core-reader/03-async-slab-source.md
        todo!("IoUringSlabSource requires the io-uring crate + Linux CI validation")
        // TODO.impl/03-core-reader/03-async-slab-source.md
    }
}

impl SlabSource for IoUringSlabSource {
    fn plaintext_for(&self, _drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, CoreError>> {
        // TODO.impl/03-core-reader/03-async-slab-source.md
        todo!("IoUringSlabSource::plaintext_for requires io_uring crate integration")
        // TODO.impl/03-core-reader/03-async-slab-source.md
    }

    fn slab_count(&self) -> usize {
        todo!() // TODO.impl/03-core-reader/03-async-slab-source.md
    }

    fn drop_count(&self) -> usize {
        todo!() // TODO.impl/03-core-reader/03-async-slab-source.md
    }
}
