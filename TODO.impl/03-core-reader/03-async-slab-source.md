# 03 — Async slab source trait

- **Status:** pending (trait + sync impl this cycle; io_uring / kqueue later)
- **Phase:** 2
- **Depends on:** 03-drop-store-reader
- **Design refs:** §5, 2026-throughput-roadmap.md §5
- **Priority:** P1

## Goal

`SlabStore::plaintext_for` is the only read path; it's sync,
memory-mapped, and uses the kernel page cache. That's the right
default. For workloads where it isn't (random read on cold slabs,
batch prefetch of N drops, OS-level submission queue batching), we
need a trait that lets a future `io_uring`-backed impl slot in
without rewriting every caller.

## Design

```rust
#[async_trait::async_trait(?Send)]
pub trait SlabSource: Send + Sync {
    async fn read_drop(&self, drop_id: &DropId) -> Result<Vec<u8>, CoreError>;
    async fn contains(&self, drop_id: &DropId) -> bool;
    fn slab_count(&self) -> usize;
    fn drop_count(&self) -> usize;
}

pub struct MmapSlabSource { /* wraps SlabStore */ }
pub struct IoUringSlabSource { /* cfg(linux); gated on `io-uring` feature */ }
```

The sync `MmapSlabSource` delegates to today's `SlabStore` and
returns `async { ready }`. No `tokio` runtime required (the trait
is `?Send`, future-less polling).

## Notes

- `async-trait` is the only new dep, and only if we land the trait
  this cycle. The simpler alternative is a sync trait with a
  `Poll`-style API; we adopt `async-trait` to leave room for real
  futures.
- macOS kqueue doesn't give us submission queues the way io_urning
  does; on macOS the trait's win comes from `posix_fadvise` and
  `readv(2)` batching, not from kqueue itself.

## Acceptance

- [ ] `SlabSource` trait exists in `limnifs-core::slab_source`.
- [ ] `MmapSlabSource` wraps `SlabStore` and round-trips in tests.
- [ ] `SlabStore` keeps its current API for backward compat; the
      trait lives alongside.
- [ ] `IoUringSlabSource` is a stub with `todo!()` and a feature
      flag, documented as Linux-only.
