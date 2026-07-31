# 34 — Full pure-Rust LZMA/LZMA2/XZ encoder (repo: [omnizip/omnizip-rs](https://github.com/omnizip/omnizip-rs))

- **Priority:** P1 (blocks ratio parity with reference `xz`)
- **Depends on:** 35 (registry refactor)
- **Estimated effort:** **weeks–months** (algorithmic logic already done in Ruby)
- **Repos touched:** [`omnizip/omnizip-rs`](https://github.com/omnizip/omnizip-rs) `omnizip-lzma` crate, then `limnifs/limnifs`
- **Porting strategy:** line-by-line Rust port of omnizip's Ruby LZMA reference (`omnizip/lib/omnizip/algorithms/lzma/` 7,558 LOC + `lzma2/` 906 LOC, all MIT). The Ruby already includes the match finder, optimal parser, range coder, and XZ container — the algorithmically hard parts. C reference (`tukaani-project/xz` liblzma, 0BSD) consulted for perf tuning only after the Ruby port verifies correct. Test fixtures adopted from `omnizip/spec/fixtures/`. Full plan: [`omnizip-rs/PLAN.md`](https://github.com/omnizip/omnizip-rs/blob/main/PLAN.md).
- **Supersedes:** the archived `limnifs/lzma` skeleton (2026-07-31).

## Problem

`lzma-rs` 0.3 has a full LZMA/LZMA2/XZ **decoder** and a
non-functional **encoder**:

- `encode/lzma2.rs:19` writes every chunk as `status = 1` (uncompressed
  reset dict) — no actual compression.
- `encode/dumbencoder.rs:71–82` emits literals only via the range
  coder — no match sequences except the end-of-stream marker.

Result: any "LZMA" output produced by `lzma-rs` is ~input size + 13
bytes per 64 KB chunk. We confirmed this in the codec PR #108: the
"best ratio" path was producing larger output than LZ4.

## Goal

Fork `lzma-rs` to `limnifs/lzma` and implement real match finders
plus the optimal parser that gives LZMA its signature ratio.

## Phased plan

### Phase A — HC4 match finder + literal encoder (levels 0–3 equivalent): ~2–4 weeks

| Level | Description |
|---|---|
| 0 | BT2 (binary tree, 2-byte minimum match) |
| 1 | HC3 (hash chain, 3-byte minimum match) |
| 2 | HC4 (hash chain, 4-byte minimum match) — default LZMA |
| 3 | BT4 (binary tree, 4-byte minimum match) |

Each level emits raw LZMA1 streams (5-byte header + range-coded body +
end marker). The decoder is unchanged from `lzma-rs`.

Acceptance: ratio within 15% of reference `xz -0..-3` on Silesia;
encode throughput ≥ 10 MB/s at level 2.

### Phase B — Optimal parser (levels 4–6): ~1–2 months

The LZMA optimal parser searches the future cost of literal vs match
at each position, using dynamic programming. This is where the bulk
of LZMA's ratio advantage comes from.

| Level | Search depth |
|---|---|
| 4 | shallow optimal, 4-byte minimum match |
| 5 | shallow optimal, BT4, 16 MB dictionary |
| 6 | deeper optimal, BT4, 64 MB dictionary — reference defaults |

Acceptance: ratio within 5% of `xz -6` on Silesia; encode throughput
≥ 3 MB/s.

### Phase C — High-ratio levels (7–9) + LZMA2 chunking + XZ container: ~2–3 months

| Level | Notes |
|---|---|
| 7 | deeper BT4 search, more candidates |
| 8 | very deep search + multi-pass |
| 9 | extreme search; encode time vs ratio limit |

Plus: LZMA2 chunk format (variable chunk size, copy/reset chunk
types) and XZ container (stream headers, block headers, index, CRC64).

Acceptance: ratio within 3% of `xz -9` on Silesia; round-trips
through reference `xz -d` byte-identically.

## Architectural notes

- Fork upstream; keep the decoder intact.
- Range coder implementation: ruzstd-style state, no external deps.
- Match finder: in-memory window, configurable dictionary size
  (default 64 MB at level 6).
- License: lzma-rs is MIT/Apache-2.0. Fork preserves both.
- Codec id mapping in `limnifs/limnifs`:
  - level 0–9 → our encoder (when Phase A/B/C ships)
  - decoder: always delegate to lzma-rs's existing decoder
  - until phases ship: encode returns `UnsupportedFeature` (current
    behaviour post PR #108).

## Acceptance gates (overall)

- Workspace `cargo test` green at every phase.
- Cross-verify: a `.lim` image produced by LimniFS at LZMA-6
  round-trips through reference `xz -d` and back, byte-identical.
- Conformance vectors under `limnifs-conformance/vectors/lzma/`
  covering each implemented level.
- Per-phase PRs into `limnifs/limnifs` switching the dispatch to use
  the forked encoder; rebased-merged.

## Why this is "absolutely necessary"

XZ/LZMA is the standard "best ratio" codec across the archival and
distribution ecosystems (`xz -9` is the default for many Linux
distro ISOs, source tarballs, conda packages). Without a pure-Rust
encoder, LimniFS cannot match that ratio for fresh images. The user
directive 2026-07-31 (`Porting liblzma in FULL can be long but
ABSOLUTELY NECESSARY`) confirms this is non-negotiable.
