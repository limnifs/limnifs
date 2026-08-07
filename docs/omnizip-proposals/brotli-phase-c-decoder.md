# omnizip-brotli: compress_fragment output rejected by in-house decoder

- **omnizip version:** 0.14.40 (local / tagged, not yet on crates.io as of check)
- **LimniFS version:** 0.2.40 (workaround: tests relaxed)
- **Filed:** 2026-08-07
- **Status:** Open — Phase C partial

## Summary

omnizip-brotli 0.14.40 ships an in-house encoder (Phase C partial):
quality 0–1 → `fast_encoder::vendored_compress`, quality 2–11 →
`compress_fragment::compress`. Intermediate qualities do **not**
differentiate ratio (both 2 and 11 call the same `compress_fragment`
path with no quality argument).

On highly-repetitive inputs (e.g. `"The quick brown fox. " × 10000`),
`compress_fragment` produces a stream that the in-house decoder
rejects with:

```
brotli decode failed: repeat overflows alphabet
```

Milder inputs (shorter Lorem-style repeats) round-trip fine. The
upstream C reference (`brotli -d`) is reported to accept the encoder
output across 11 fixtures, so the bug is decoder-side relative to
the new encoder's edge cases.

## LimniFS impact

- Balanced profile uses Brotli q5 for text → `compress_fragment`.
- Round-trip on normal source trees still works (existing
  `brotli_round_trips` / FSST+Brotli tests pass).
- Pathological synthetic text may fail extract if compressed with
  q≥2. Until fixed, prefer ZSTD for max-read / competitive profiles
  (already the case for several profiles).

## Acceptance

1. `"The quick brown fox. " × 10000` round-trips through
   `BrotliCodec` at quality 5 and 11.
2. Quality 0 produces larger (or equal) output than quality 11 on
   mixed natural-language text of ≥100 KB (when full quality
   differentiation lands).
3. LimniFS `brotli_and_zstd_both_compress_text` can restore a hard
   round-trip on the pathological input.
