# 01 — Metadata blob: zstd instead of Brotli

- **Priority:** P0
- **Side:** LimniFS
- **Est. effort:** 1h

## Problem

Benchmark `tiny-files` dataset: LimniFS 66.87% ratio vs SquashFS 42.78%.
SquashFS compresses its inode/metadata table with zstd; LimniFS uses
Brotli q5 (small blobs) or q2 (large blobs). zstd L3 is faster than
Brotli q5 on metadata-shaped data (repetitive u64/u32 fields) and
produces comparable or better ratio.

## Fix

Change `WriteConfig::defaults.metadata_codec` from `"brotli"` to
`"zstd"` in all profiles except `max-ratio` (where Brotli q11 wins
on ratio). Also change `metadata_quality` from 5 to 3 (zstd L3 is
the standard fast level).

## Expected impact

- **tiny-files ratio**: ~60% → ~50% (closer to SquashFS 42.78%).
  The metadata blob is a significant fraction of tiny-files images.
- **Create speed on large trees**: faster metadata compression.
- **No impact** on data drops (only the metadata blob codec changes).

## Findings (2026-08-20, measured on real metadata blobs)

- [x] Benchmark — REJECTED on ratio: on structured metadata (2.3 MB
      blob) Brotli q2 = 97.26% vs zstd = 99.74%; Brotli wins where the
      codec choice matters. On inline-dominated blobs both are
      content-bound (99.99%). zstd wins only SPEED (6.5x: 520 ms vs
      3.4 s on a 43.6 MB blob) — that gap is omnizip's dictionary-
      lookup issue (BUGREPORT-brotli-dict-lookup-O(n).md upstream),
      and per the wait-for-upstream rule we do not change codec
      profiles to dodge it. No profile change.
