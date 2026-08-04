# 07 — Parallel slab assembly

- **Priority:** P2
- **Side:** LimniFS
- **Est. effort:** 2d

## Problem

After parallel compress, `WriteContext::assemble` packs drops into
the slab's solid window sequentially. Each drop's compressed bytes
are copied into the window one at a time. For images with 100K+
drops, this sequential copy takes measurable time.

## Fix

Split the slab window into N chunks (one per rayon worker). Each
worker copies its assigned drops' compressed bytes in parallel.
Concatenate the chunks. The drop records are assembled in parallel
too (each record is 49 bytes of fixed-format encoding).

## Expected impact

- Marginal (slab assembly is fast today, ~50ms on 100K drops).
- Matters only for extreme-scale images (1M+ drops).

## Acceptance

- [ ] Slab window built in parallel.
- [ ] No change in output bytes.
- [ ] Benchmark: large-image create speed improves measurably.
