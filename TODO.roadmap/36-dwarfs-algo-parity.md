# 36 — DwarFS algorithm parity matrix

- **Priority:** P1 umbrella (blocks the "beat DwarFS on ratio" claim at high levels)
- **Depends on:** 31 (brotli), 32 (deflate), 33 (full zstd), 34 (full lzma), 06 (codec map flag), 07 (classifier)
- **Estimated effort:** serialises the codec work above

## Problem

DwarFS's compression portfolio is:

| Codec | DwarFS levels | LimniFS pure-Rust status |
|---|---|---|
| LZ4 | 1 | ✅ full (lz4_flex) |
| ZSTD | 1–22 | ⚠️ level 1 only (ruzstd) — see 33 |
| LZMA/XZ | 0–9 | ❌ decode-only (lzma-rs) — see 34 |
| Brotli | 0–11 | ❌ not wired — see 31 |
| DEFLATE | gzip compatibility | ❌ not wired — see 32 |

We beat DwarFS today on the **default** pipeline (create, extract,
size) because our parallel writer + seine classifier + ZSTD-1 beats
DwarFS's default config. But the **ratio** comparison at high levels
is not winnable until the codec gaps close.

## Goal

Close every algorithm gap and document a parity matrix in
`TODO.impl/04-writer-pipeline/04-dwarfs-parity.md`:

| Codec | DwarFS | LimniFS | Ratio gap | Status |
|---|---|---|---|---|
| LZ4 | -1 | -fast | 0% | ✅ parity |
| ZSTD-1 | -1 | zstd/Fastest | 0% | ✅ parity |
| ZSTD-6 | -6 | (blocked on 33-B) | TBD | 🟡 Phase B |
| ZSTD-19 | -19 | (blocked on 33-C) | TBD | 🟡 Phase C |
| LZMA-6 | -6 | (blocked on 34-B) | TBD | 🟡 Phase B |
| LZMA-9 | -9 | (blocked on 34-C) | TBD | 🟡 Phase C |
| Brotli-11 | -11 | brotli/11 | TBD | 🟢 pending 31 |
| DEFLATE-9 | -9 | deflate/9 | TBD | 🟢 pending 32 |

## Per-class codec policy

Once all codecs are wired, the classifier (07) routes each content
class to the best codec via the user-configurable codec map (06):

| Seine class | Default codec | Rationale |
|---|---|---|
| Text | ZSTD-3 (Phase A) or Brotli-11 (deepening) | best text ratio |
| Code | Brotli-11 | outperforms LZMA on source code |
| Binary structured | ZSTD-6 | structured redundancy favours ZSTD |
| Binary random | store | no codec helps; skip CPU |
| Media | store | already entropy-coded |
| Archive | store | double compression wastes CPU |

User can override via `--codec-map Text=zstd-6,Code=brotli-11,…`.

## Acceptance gates

- For each codec in the matrix above, a conformance vector under
  `limnifs-conformance/vectors/` with:
  - 1 MB sample from Silesia corpus
  - round-trip byte-identical
  - ratio within the target gap of the reference encoder
- The `benchmarks/run_benchmarks.py` suite reports LimniFS-vs-DwarFS
  ratios at LZMA-6, ZSTD-6, Brotli-11.
- `STATUS.md` records the headline number for each tier.
