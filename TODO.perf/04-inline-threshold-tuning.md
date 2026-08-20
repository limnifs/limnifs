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

## Resolution (2026-08-20)

- [x] Profiles use per-profile `inline_threshold` — the config knob
      was silently IGNORED (walk + process_file read the hard constant
      `INLINE_THRESHOLD`); now threaded through WriteContext into both
      decision points. Profile values (4096 / 8192) now take effect.
- [x] Benchmark on tiny-files shows ratio improvement — REJECTED with
      data (50K unique 1 KiB files): threshold 4096 (inline) = 87.1%
      total, 4.6 s create; threshold 512 (slab path) = 108.3% total,
      68.4 s create. Inline wins BOTH ways: the shared-inline table
      already dedups duplicate inline files, and one metadata stream
      compresses tiny files better than 50K independent chunks (which
      also pay full tournament cost per chunk). Default thresholds
      unchanged.
