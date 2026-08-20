# 18 — mmap on the output side

- **Priority:** P2
- **Side:** LimniFS
- **Est. effort:** 6h

## Problem

`SlabArtifact::bytes: Vec<u8>` is materialized in memory then written
via `std::fs::write`. For multi-GiB images with N slabs, peak RSS is
`sum(slab_sizes)` at write time. We've already mmap'd the INPUT side
(v0.2.30); the output side is symmetric.

## Fix

For slabs above a threshold (e.g. 64 MiB), allocate the file first,
mmap it, write the slab bytes via the mapping, then `msync`+drop.
Below the threshold, plain `std::fs::write` is faster (no VMA setup).

The crossover probably lives around 64 MiB on Linux, higher on macOS
due to Pageable Object Cache differences.

## Expected impact

- Peak RSS drops from `sum(slab_sizes)` to `max(slab_size)` for
  large images.
- Wall-clock similar (mmap write is not faster than buffered write).

## Findings (2026-08-20)

- [x] mmap-output path for slabs > 64 MiB — N/A: slabs are capped at
      MAX_SLAB_TOTAL_BYTES (the reader's 64 MiB ceiling), so a
      >64 MiB slab cannot exist post-pivot. The TODO's premise
      predates the wire-format pivot.
- [ ] Peak RSS measurement on multi-GiB image — REDIRECTED: the real
      resident-memory duplication is drops (Arc compressed bytes)
      PLUS the per-slab solid-window copies built by pack_slabs
      (~2x compressed content at peak). Fixing that means streaming
      slab artifacts to disk instead of returning Vec-backed
      SlabArtifact bytes — an artifact-API redesign, tracked as
      future work, not an mmap swap.
- [x] Output bytes unchanged — no code path changed.
