# 04 — Per-profile inline_threshold tuning

- **Priority:** P1
- **Side:** LimniFS
- **Est. effort:** 1h

## Problem

All profiles use `inline_threshold = 4096`. Files ≤ 4 KiB go inline
(stored raw in the inode, no slab chunking). For tiny-files
workloads (50K × 1 KiB), this means every file is inline. The
metadata blob grows to ~10 MB of un-compressed-inline data.

Lowering `inline_threshold` for `balanced` and `max-ratio` profiles
forces tiny files through the slab path where per-class dictionary
training + zstd compression applies.

## Fix

- `balanced()`: `inline_threshold = 1024` (1 KiB).
- `max-ratio()`: `inline_threshold = 512`.
- `max-write()`: keep 4096 (inline is fastest for write).
- `max-read()`: `inline_threshold = 8192` (more inline = fewer slab
  reads during extract).

## Expected impact

- **tiny-files ratio**: 66.87% → ~50% (files go through dict-trained
  zstd instead of raw inline).
- **tiny-files create speed**: slightly slower (chunking overhead on
  tiny files). Acceptable for balanced/max-ratio.

## Acceptance

- [ ] Profiles use per-profile `inline_threshold`.
- [ ] Benchmark on tiny-files shows ratio improvement.
