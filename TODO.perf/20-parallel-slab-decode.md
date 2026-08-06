# 20 — Parallel slab decode (read side)

- **Priority:** P1
- **Side:** LimniFS
- **Est. effort:** 1d

## Problem

`extract` and `read_drop` paths decode slabs sequentially. Each slab's
drop records are parsed in a loop, decompression happens per-drop.
For large images with many slabs, parallel slab decode could speed
up extract noticeably.

## Fix

When extracting N drops across M slabs, rayon across slabs: each
worker decodes its assigned slab, returns decoded drops. Caller
merges in order.

Already partially done via `ParallelExtractSink` (parallel extract
across drops). This TODO extends to parallel decode of the slab
metadata itself (drop records parse).

## Expected impact

- 1.5–3× on extract of large images
- Already-fast on small images

## Acceptance

- [ ] Slab decode parallelized via rayon
- [ ] Output bytes unchanged (decode is deterministic)
- [ ] Benchmark: extract improves measurably
