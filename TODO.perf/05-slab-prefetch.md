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

- [ ] `SlabStore::load_mmap` calls `posix_fadvise` (Linux) or
      `madvise` (macOS) after mmap.
- [ ] Benchmark on cold cache (drop page cache before run) shows
      improvement.
