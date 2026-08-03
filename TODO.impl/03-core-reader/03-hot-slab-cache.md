# 03 — Hot slab cache (LRU over decoded drops)

- **Status:** pending
- **Phase:** 2
- **Depends on:** 03-drop-store-reader
- **Design refs:** §5, 2026-throughput-roadmap.md §4
- **Priority:** P1

## Goal

`SlabStore::plaintext_for` decompresses on every call. For
read-heavy workloads (mount, `cat-multi`, RW turnover on a hot
image), the same drops are decompressed over and over. A small LRU
cache over recently-decoded plaintext gives ~10× speedup on the
second access.

## Design

```rust
pub struct CachedSlabStore<S: SlabSource> {
    inner: S,
    cache: Mutex<LruCache<[u8; 32], Vec<u8>>>,
}

impl<S: SlabSource> CachedSlabStore<S> {
    pub fn new(inner: S, capacity: usize) -> Self { /* ... */ }
}
```

- Cache key: full `DropId` (BLAKE3 of plaintext, 32 bytes).
- Cache value: owned `Vec<u8>` plaintext.
- Capacity in entries, default 256 (≈ 4 MiB at 16 KiB avg drop).
- Thread-safe via `Mutex<LruCache>`; contention is negligible at
  this granularity.

## Notes

- Mount use case is the win; the FUSE daemon will read the same
  drops many times in a `find /mnt -exec grep`.
- Don't cache compressed bytes — only plaintext. Compressed bytes
  are already mmap'd, the kernel page cache handles that.

## Acceptance

- [ ] `CachedSlabStore` wraps any `SlabSource` and round-trips in
      tests.
- [ ] A benchmark in `limnifs-bench` measures cache hit ratio and
      decompress count.
- [ ] `cat-multi` of a 1000-file tree runs ≥ 3× faster on the
      second invocation.
