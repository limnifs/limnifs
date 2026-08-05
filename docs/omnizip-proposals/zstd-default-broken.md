# omnizip-zstd: Default (L6) regression on highly-repetitive inputs

- **omnizip version affected:** 0.14.8, 0.14.9
- **omnizip version fixed:** 0.14.10 (omnizip-rs PR #90)
- **LimniFS versions affected:** 0.2.21 (workaround in place)
- **LimniFS versions fixed:** 0.2.23 (workaround removed)
- **Filed:** 2026-08-05
- **Status:** **RESOLVED upstream** — 2026-08-05

## Summary

`omnizip_zstd::compress` at `ZstdLevel::Default` (L6), `Better`
(L12), and `Best` (L22) produced pathological output on
highly-repetitive inputs in omnizip 0.14.8/0.14.9: 500–700× larger
than `Fastest` (L1) and 70,000× slower.

`Fastest` (L1) and `Fast` (L3) were unaffected.

## Resolution

omnizip-rs PR #90 (released in 0.14.10) restores correct level
differentiation. LimniFS 0.2.23 removes the L3 cap in
`level_for_quality` and restores the L1-vs-L6 regression test.

## Historical reproduction (0.14.8)

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

`Default` and `Better` produced identical 50,842-byte output —
strongly suggesting the encoder fell back to raw/uncompressed
block mode for this input at those levels.

## Historical side effects on LimniFS (0.2.21)

Three correctness tests in `limnifs-core/src/codec/mod.rs` failed:

- `zstd_higher_levels_compress_better_than_lower` — L6 vs L1
- `zstd_compresses_binary_data` — sequential-byte pattern, L6
- `zstd_compresses_better_than_lz4_on_text` — L6 vs LZ4

Full workspace test suite runtime: **367 seconds** (3 ZSTD tests
eating ~99% of wall time).

## Workaround in LimniFS 0.2.21

`limnifs-core/src/codec/zstd.rs::level_for_quality` capped every
requested level at `Fast` (L3). Decompression was unaffected —
ZSTD's wire format is level-independent.

## Acceptance criteria (all met)

1. ✅ `omnizip_zstd::compress(input, ZstdLevel::Default)` produces
   output ≤ `ZstdLevel::Fast` output for any input where `Fast`
   output < input.
2. ✅ `compress` at `Default`/`Better`/`Best` completes in ≤ 10× the
   time of `Fast` on inputs under 1 MiB.
3. ✅ The three LimniFS tests pass with the original L1-vs-L6
   comparison.

The L3 cap in `level_for_quality` and the L1-vs-L3 test variant
were both reverted in LimniFS 0.2.23.
