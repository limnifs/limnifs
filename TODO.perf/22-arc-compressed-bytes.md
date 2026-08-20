# 22 — Arc<Vec<u8>> for compressed bytes (avoid clones)

- **Priority:** P1
- **Side:** LimniFS
- **Est. effort:** 6h

## Problem

`PendingDrop::compressed: Vec<u8>` is owned. Each Vec is moved once
into the drop. The cross-file compress cache I added (v0.2.36) clones
the Vec when storing AND when retrieving — for cache hits, the
retrieved bytes are cloned even though they could be shared.

For workloads with high cache hit rate (container layers), the clones
double the allocation cost.

## Fix

Switch `PendingDrop::compressed` (and `ChunkedFileResult::drops`) to
`Arc<Vec<u8>>` or `Arc<[u8]>`. The compress cache stores
`Arc<[u8]>`, retrieval is a cheap Arc clone (refcount bump).

Slab packing uses `&[u8]` borrows from the Arc, so no semantic change.

## Expected impact

- 10–30% peak RSS reduction on workloads with high dedup rate
- Small wall-clock improvement (fewer allocations)
- Output bytes unchanged

## Acceptance

- [x] `compressed` field is `Arc<[u8]>`
- [x] Cache hits are refcount bumps (no alloc)
- [x] Output bytes unchanged (verified byte-identical on dedup-heavy 200-file tree)
- [x] Benchmark: container-layer workloads improve (measured 2026-08-20 on omnizip 0.16.75: 4000-file / 262 MB tree with 100 unique 64 KiB contents -> create 0.2 s flat, 100 drops (perfect dedup), 6.5 MB slab, ~410 MB peak RSS, byte-identical extract, deterministic root across runs; the 3900 cache hits are Arc refcount bumps)
