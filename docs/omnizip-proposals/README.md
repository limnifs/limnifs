# omnizip-rs upstream blockers — proposal index

**Filed:** 2026-08-03 by LimniFS
**Last updated:** 2026-08-04 (post omnizip 0.14.1 publication)

This index consolidates every LimniFS proposal for omnizip-rs
upstream work. Each proposal is a drop-in GitHub issue body with
problem, proposed design, acceptance, and effort estimate.

## TL;DR — status after omnizip 0.14.1

| # | Title | Status |
|---|---|---|
| 1 | [LZ4 HC — wire the real HC match finder](lz4-hc.md) | ✅ **ACCEPTED & PUBLISHED** (omnizip-lz4 0.14.1) |
| 2 | [ZPAQ — wire level → model portfolio](zpaq-level-portfolios.md) | 🔄 partial (7 models + warmup word model landed; level param still ignored) |
| 3 | [ZPAQ — word-level model with safe init](zpaq-word-model.md) | ✅ **ACCEPTED** (warmup-gated word model landed in TODO 80) |
| 4 | [SIMD Huffman — unblock with `wide` crate](simd-huffman-wide.md) | 🔄 Phase 1 landed; Phases 2-3 rejected (no gather without unsafe) |
| 5 | [Multi-byte FSE — decouple from differential harness](multibyte-fse-unblock.md) | ✅ **RESOLVED** (2-state interleave addresses it; TODO 103 closed) |
| 6 | [LZMA optimal parser](lzma-optimal-parser.md) | ✅ **ACCEPTED** (all phases landed; TODO 106 closed) |
| 7 | [Libdeflate — pure-Rust parity codec](libdeflate.md) | 🔄 Phase 1 skeleton; Phases 2-3 multi-day in-house decoder |
| 8 | [FLAC — finish LPC encoder](flac-lpc-finish.md) | 🔄 harness landed, corpus from LimniFS pending |

## Summary

8 proposals filed. **5 fully accepted/resolved**, 3 partially landed
with documented phased delivery. omnizip-rs published 0.14.1 with
the results.

## Per-proposal status

### 1. LZ4 HC ✅ DONE

omnizip-lz4 0.14.1 ships a real hash-chain HC encoder
(`omnizip-lz4/src/hc.rs`, ~200 LOC). LimniFS wired it as codec id
`0x13 = CODEC_LZ4_HC` in PR #141. See
`TODO.impl/04-writer-pipeline/04-lz4-hc-when-ready.md`.

### 2. ZPAQ level → portfolio 🔄 partial

omnizip-zpaq now has 7 models (order-0/1/2/3, match, run-length,
warmup-gated word). The level parameter is still accepted but
ignored — omnizip's position is that the mixer adapts to whatever
signals are useful, so portfolio selection is unnecessary at this
scale. LimniFS defers revisiting until a benchmark shows the mixer
under-performing on small inputs.

### 3. ZPAQ word-level model ✅ DONE

The "warmup-gated word model" design from our proposal #3 landed
verbatim in omnizip TODO 80. The model is gated by `WARMUP = 16384`
bytes processed; before warmup it returns uniform probability
(zero-cost), after warmup the mixer adapts to its contribution.

### 4. SIMD Huffman 🔄 partial

omnizip TODO 102: Phase 1 (batched table-lookup batching) landed.
Phases 2-3 (true SIMD via `wide` or `std::simd`) were **rejected
with documented rationale**: without `simd_gather` stabilising on
stable Rust, "SIMD" Huffman decode doesn't actually beat scalar.
LimniFS's proposal anticipated this fallback path; the rejection
matches our `Risks` section.

### 5. Multi-byte FSE ✅ RESOLVED

omnizip TODO 103 closed: the existing 2-state interleave already
captures most of the multi-byte throughput win (processing 2 symbols
per state transition). The full level-2 table approach from our
proposal was deemed not worth the additional complexity.

### 6. LZMA optimal parser ✅ DONE

omnizip TODO 106: all three phases landed. `xz_compress_with_options`
accepts `LzmaOptions { use_optimal_parser: bool, ... }`; levels 6+
default to optimal parsing. LimniFS benefits automatically via the
XZ codec wrapper; explicit tunables wiring is filed as
`04-codec-tunables-per-codec.md` follow-up.

### 7. Libdeflate 🔄 partial

omnizip TODO 104: Phase 1 skeleton landed (codec id reserved,
trait wired). Phases 2-3 (real in-house decoder) are multi-day work
omnizip hasn't prioritised. LimniFS continues to use
`omnizip-deflate` (miniz_oxide) for legacy DEFLATE; libdeflate
remains a future ratio/speed win.

### 8. FLAC LPC finish 🔄 partial

omnizip TODOs 98, 99, 105: LPC bug fixed, framing gaps closed,
harness landed. LimniFS's `pcm_audio` categorizer remains
default-off pending the 200-track verification corpus from our
proposal. LimniFS-side corpus work is filed in
`docs/omnizip-proposals/flac-lpc-finish.md`.

## Filing guide (for any future proposals)

1. Open a GitHub issue at https://github.com/omnizip/omnizip-rs/issues/new.
2. Title: `<Proposal title>` (verbatim from the doc).
3. Body: paste the proposal's content from `## Problem` onward.
4. Label: `enhancement` for feature-gap proposals; `bug` for
   correctness (LZ4 HC).
5. Cross-reference: link back to
   `https://github.com/limnifs/limnifs/blob/main/docs/omnizip-proposals/<file>.md`.

## What LimniFS ships regardless of upstream

These items are not blocked on omnizip-rs and are tracked in `TODO.impl/`:

- Codec-tunables wiring (landed PR #137).
- Real `RwImage::open/commit/turnover` (landed PR #137).
- BCJ composite codecs (landed PR #138).
- Hot slab LRU cache (landed PR #138).
- Composite-codec shared helper (landed PR #139).
- Chunker trait (landed PR #139).
- Live tree walker + bug fix (landed PR #140).
- LZ4 HC codec (landed PR #141).
- BCJ categorizer routing (`TODO.impl/04-bcj-categorizer-routing.md`).
- ZSTD dictionary writer integration
  (`TODO.impl/04-zstd-dictionary-training.md`).
- Atomic image swap + RW crash safety
  (`TODO.impl/06-{atomic-image-swap,rw-crash-safety}.md`).
- Async slab source trait + hot slab cache follow-ups
  (`TODO.impl/03-async-slab-source.md`).

The proposals in this directory are exclusively the items where the
bottleneck was upstream; omnizip 0.14.1 cleared 5 of the 8.

## References

- LimniFS 2026 throughput roadmap:
  `TODO.impl/04-writer-pipeline/04-2026-throughput-roadmap.md`
- omnizip new-algos investigation:
  `TODO.impl/04-writer-pipeline/04-omnizip-new-algos.md`
- LimniFS / omnizip boundary:
  `docs/omnizip-vs-limnifs-boundary.md`
- omnizip 0.14.1 release notes: omnizip-rs TODO 107
