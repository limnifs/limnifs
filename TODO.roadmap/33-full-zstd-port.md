# 33 — Full pure-Rust ZSTD encoder (repo: [omnizip/omnizip-rs](https://github.com/omnizip/omnizip-rs))

- **Priority:** P1 (blocks "best ratio" tier without it)
- **Depends on:** 35 (registry refactor), 31 (codec 0x04 brotli — for ratio comparison)
- **Estimated effort:** **weeks–months** (algorithmic logic already done in Ruby)
- **Repos touched:** [`omnizip/omnizip-rs`](https://github.com/omnizip/omnizip-rs) `omnizip-zstd` crate, then `limnifs/limnifs`
- **Porting strategy:** line-by-line Rust port of omnizip's Ruby ZSTD reference (`omnizip/lib/omnizip/algorithms/zstandard/`, 3,150 LOC, MIT). C reference (`facebook/zstd`, BSD-3-Clause) consulted for perf tuning only after the Ruby port verifies correct. Test fixtures adopted from `omnizip/spec/fixtures/`. Full plan: [`omnizip-rs/PLAN.md`](https://github.com/omnizip/omnizip-rs/blob/main/PLAN.md).
- **Supersedes:** the archived `limnifs/zstd` skeleton (2026-07-31).

## Problem

`ruzstd` 0.9 has a full ZSTD decoder and only a `Fastest` (level 1)
encoder. Levels 2–22 are marked `UNIMPLEMENTED` in
`encoding/mod.rs:50–66`. Without a full encoder, LimniFS cannot
match ZSTD's ratio curve at the levels that DwarFS, ZFS, BTRFS, and
modern container images rely on.

## Goal

Fork `ruzstd` to `limnifs/zstd` and implement the missing encoder
levels. Phase the work so each level is independently shippable and
the ratio gap closes incrementally.

## Phased plan

### Phase A — Levels 2–3 (greedy and lazy matchers): ~2–4 weeks

| Level | Matcher | Notes |
|---|---|---|
| 2 | greedy hash-chain (HC3) | adds a small hash chain beyond ruzstd's window-1 single-match |
| 3 | lazy matching | one-lookahead best-match-or-skip decision |

Acceptance: ratio within 10% of reference zstd -2 / -3 on the Silesia
corpus; encode throughput ≥ 50 MB/s on a single core.

### Phase B — Levels 4–9 (optimal parsing): ~1–2 months

| Level | Parser | Notes |
|---|---|---|
| 4 | optimal parser, small window (4 KB) | short matches, dictionary probes |
| 5 | optimal parser, larger window (8 KB) | better ratio for medium files |
| 6 | default reference encoder parameters | parity target |
| 7–9 | tuned optimal parser, larger search depth | ratio gains from more candidates |

Acceptance: ratio within 5% of reference zstd -6 / -9 on Silesia;
encode throughput ≥ 20 MB/s at level 6.

### Phase C — Levels 10–22 (large window, ultra mode, dictionary): ~2–4 months

| Level | Feature |
|---|---|
| 10–12 | 128 KB window, BT4 binary-tree match finder |
| 13–15 | 256 KB–1 MB window, multi-threaded match search |
| 16–19 | 4 MB–16 MB window, ultra mode (greedy + lazy + optimal mix) |
| 20–22 | 32 MB+ window, full dictionary training integration |

Acceptance: ratio within 3% of reference zstd -22 on Silesia; encode
throughput ≥ 5 MB/s at level 19.

## Architectural notes

- Fork `ruzstd` upstream; keep the decoder intact (the reader side is
  already production-grade).
- The encoder must write standard ZSTD frames so that any conformant
  ZSTD decoder (including C `libzstd`) can read LimniFS-produced
  streams.
- License: ruzstd is MIT/Apache-2.0. Fork preserves both.
- Codec id mapping in `limnifs/limnifs`:
  - level 1 → ZSTD codec 0x02 Fastest (current ruzstd behaviour)
  - levels 2–3 → ruzstd + new encoders (Phase A)
  - levels 4–9 → our encoders (Phase B)
  - levels 10–22 → our encoders (Phase C)
  - decoder: always delegate to ruzstd's StreamingDecoder for all
    levels.

## Acceptance gates (overall)

- Workspace `cargo test` green at every phase.
- Cross-verify: a `.lim` image produced by LimniFS at ZSTD-19 round-
  trips through reference `zstd -d` and back, byte-identical.
- New conformance vectors under `limnifs-conformance/vectors/zstd/`
  covering each implemented level against the Silesia + enwik9 corpora.
- Per-phase PRs into `limnifs/limnifs` switching the codec dispatch to
  use the forked encoder; rebased-merged.

## Why this is "absolutely necessary"

LimniFS's benchmark numbers (see `STATUS.md` session 27) beat DwarFS
at ZSTD-1 default levels. To extend that lead to the ratio-sensitive
archival tier — and to match DwarFS's algorithm portfolio without
relying on the C `libzstd` — a full encoder is required. The user
directive 2026-07-31 (`Porting libzstd in FULL can be long BUT
ABSOLUTELY NECESSARY`) confirms this is non-negotiable.
