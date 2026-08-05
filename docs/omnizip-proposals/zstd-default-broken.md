# omnizip-zstd: Default (L6) regression on highly-repetitive inputs

- **omnizip version:** 0.14.8 (all 17 crates)
- **LimniFS version:** 0.2.21 (workaround in place)
- **Filed:** 2026-08-05
- **Status:** Open — awaiting upstream fix

## Summary

`omnizip_zstd::compress` at `ZstdLevel::Default` (L6), `Better` (L12),
and `Best` (L22) produces pathological output on highly-repetitive
inputs: 500–700× larger than `Fastest` (L1) and 70,000× slower.

`Fastest` (L1) and `Fast` (L3) are unaffected.

## Reproduction

Input: `b"The quick brown fox jumps over the lazy dog. ".repeat(2000)`
(90,000 bytes of highly-compressible text).

```
Level       Output bytes    Time
────────────────────────────────────
Fastest     74              176 µs
Fast        72              194 µs
Default     50,842          13.94 s
Better      50,842          95.74 s
Best        (not run — killed after 2 minutes)
```

`Default` and `Better` produce identical 50,842-byte output — strongly
suggesting the encoder falls back to raw/uncompressed block mode for
this input at those levels.

## Side effects on LimniFS

Three correctness tests in `limnifs-core/src/codec/mod.rs` failed:

- `zstd_higher_levels_compress_better_than_lower` — L6 vs L1
- `zstd_compresses_binary_data` — sequential-byte pattern, L6
- `zstd_compresses_better_than_lz4_on_text` — L6 vs LZ4

Full workspace test suite runtime: **367 seconds** (3 ZSTD tests
eating ~99% of wall time).

## Workaround in LimniFS

`limnifs-core/src/codec/zstd.rs::level_for_quality` now caps every
requested level at `Fast` (L3). The profile field
`WriteConfig::codec_tunables.zstd_quality` still accepts 1–22 for
forward compatibility, but anything above 3 collapses to L3 until
upstream fixes the encoder.

Decompression is unaffected — ZSTD's wire format is level-independent.

## Acceptance criteria (upstream)

The fix is shipped when, on `omnizip-zstd` ≥ next minor:

1. `omnizip_zstd::compress(input, ZstdLevel::Default)` produces output
   ≤ `ZstdLevel::Fast` output for any input where `Fast` output < input.
2. `compress` at `Default`/`Better`/`Best` completes in ≤ 10× the time
   of `Fast` on inputs under 1 MiB.
3. The three LimniFS tests above pass without the L1/L3 rewrite.

When that lands, restore the original
`6..=11 → Default, 12..=21 → Better, 22+ → Best` mapping in
`level_for_quality` and the L1 vs L6 comparison in
`zstd_higher_levels_compress_better_than_lower`.
