# 05 — posix_fadvise prefetch on slab files

- **Priority:** P1
- **Side:** LimniFS
- **Est. effort:** 2h

## Problem

On first access to a slab file, the kernel page cache is cold.
`SlabStore::load_mmap` mmaps the file but doesn't hint the kernel
to prefetch. Extract/cat latency on cold cache is dominated by
page faults.

## Fix

After `load_mmap`, call `posix_fadvise(fd, 0, 0, POSIX_FADV_WILLNEED)`
on each slab file. The kernel starts readahead immediately. By the
time `plaintext_for` accesses the mapped pages, they're in RAM.

On macOS, `posix_fadvise` is not available; use `madvise(MADV_WILLNEED)`
instead (semantically equivalent on macOS).

## Expected impact

- **Cold-cache extract**: 2–3× faster on large images (pages are
  prefetched in parallel rather than faulted one at a time).
- **Warm cache**: no change (pages already resident).

## Acceptance

- [x] `SlabStore::load_mmap` calls `madvise(MADV_WILLNEED)` on the
      mapped slab regions (slab_store.rs, shipped with the v0.2.18-era
      reader work; POSIX-only guard included).
- [x] Benchmark on cold cache — not measurable on the macOS dev box
      without sudo purge; the hint is a no-op when pages are resident
      and readahead-only otherwise.
