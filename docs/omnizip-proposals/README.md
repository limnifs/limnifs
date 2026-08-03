# omnizip-rs upstream blockers — proposal index

**Filed:** 2026-08-03 by LimniFS
**Status:** open — proposals ready to file as omnizip-rs GitHub issues

This index consolidates every LimniFS proposal for omnizip-rs
upstream work. Each proposal is a drop-in GitHub issue body with
problem, proposed design, acceptance, and effort estimate.

## TL;DR

| # | Title | Severity | Effort | Blocks LimniFS |
|---|---|---|---:|---|
| 1 | [LZ4 HC — wire the real HC match finder](lz4-hc.md) | correctness | 1h | `max-ratio` text path |
| 2 | [ZPAQ — wire level → model portfolio](zpaq-level-portfolios.md) | feature gap | 1-2d | profile tunability |
| 3 | [ZPAQ — word-level model with safe init](zpaq-word-model.md) | feature gap | 3d | `max-ratio` text ratio |
| 4 | [SIMD Huffman — unblock with `wide` crate](simd-huffman-wide.md) | unblock TODO 83 | 4d | ZSTD/DEFLATE decode speed |
| 5 | [Multi-byte FSE — decouple from differential harness](multibyte-fse-unblock.md) | unblock TODO 84 | 5-7d | ZSTD decode speed |
| 6 | [LZMA optimal parser](lzma-optimal-parser.md) | feature gap | 6d | `max-ratio` text parity |
| 7 | [Libdeflate — pure-Rust parity codec](libdeflate.md) | new codec | 12d | legacy DEFLATE decode |
| 8 | [FLAC — finish LPC encoder (verification corpus)](flac-lpc-finish.md) | feature gap | 2d (LimniFS side) | `pcm_audio` default-on |

## Filing guide

For each proposal above:

1. Open a GitHub issue at https://github.com/omnizip/omnizip-rs/issues/new.
2. Title: `<Proposal title>` (verbatim from the doc).
3. Body: paste the proposal's content from `## Problem` onward
   (skip the metadata header).
4. Label: `enhancement` for feature-gap proposals; `bug` for
   correctness (LZ4 HC).
5. Cross-reference: link back to
   `https://github.com/limnifs/limnifs/blob/main/docs/omnizip-proposals/<file>.md`.

## Acceptance we'll see

For each proposal omnizip-rs accepts and ships:

1. LimniFS bumps the omnizip dependency in `Cargo.toml`.
2. LimniFS wires the new feature in
   `limnifs-core/src/codec/<name>.rs` and updates the
   `WriteConfig::codec_tunables` and `CodecTunables` (in
   `limnifs-core`) as needed.
3. LimniFS adds or updates a profile to expose the feature.
4. LimniFS adds a behavioural test that proves the feature takes
   effect (the test pattern from
   `limnifs-core/src/codec/mod.rs::tests::tunables_*` is the
   template).

## What LimniFS will do regardless

These items are **not** blocked on omnizip-rs and we will ship
them on our side:

- The codec-tunables wiring (already landed — see
  `TODO.impl/04-writer-pipeline/04-ppmd-quality-wiring.md`).
- BCJ composite codecs for executable binaries
  (`TODO.impl/04-writer-pipeline/04-bcj-categorizer-routing.md`).
- Cross-image sparse index
  (`TODO.impl/04-writer-pipeline/04-cross-image-sparse-index.md`).
- Live tree walker DRY refactor
  (`TODO.impl/04-writer-pipeline/04-live-tree-walker.md`).
- RW crash safety + atomic image swap
  (`TODO.impl/06-deltas-overlays/06-{rw-crash-safety,atomic-image-swap}.md`).
- Async slab source trait
  (`TODO.impl/03-core-reader/03-async-slab-source.md`).
- Hot slab LRU cache
  (`TODO.impl/03-core-reader/03-hot-slab-cache.md`).

The proposals in this directory are exclusively the items where
the bottleneck is upstream.

## References

- LimniFS 2026 throughput roadmap:
  `TODO.impl/04-writer-pipeline/04-2026-throughput-roadmap.md`
- omnizip new-algos investigation:
  `TODO.impl/04-writer-pipeline/04-omnizip-new-algos.md`
- LimniFS / omnizip boundary:
  `docs/omnizip-vs-limnifs-boundary.md`
