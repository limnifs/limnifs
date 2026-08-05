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

- [x] Slab window built in parallel. `pack_slabs` now has two phases:
      sequential grouping (per-drop size budget — must be sequential
      because slab assignment depends on running size total), then
      parallel `encode_slab` across rayon workers.
- [x] No change in output bytes. Slab ordinals are derived from the
      position in the slab_groups vector, identical to the old
      sequential loop. All 119 limnifs-write tests pass unchanged.
- [x] Benchmark: large-image create speed improves measurably. Wall-
      clock improvement is proportional to slab count — meaningful
      only on images with many slabs (multi-GiB archives).

## Implementation notes (2026-08-05)

Shipped in v0.2.24. The change is in `limnifs-write/src/lib.rs::pack_slabs`.

The grouping phase stays sequential because slab assignment depends
on a running size budget — you can't decide which slab a drop
belongs to without knowing how full the previous slab is. The
encoding phase is fully independent per slab and parallelises
cleanly via `rayon::par_iter`.

Per-slab encoding (drop records + solid window) stays sequential
within a slab. The cross-slab parallelism is the bigger win for
extreme-scale images anyway.
