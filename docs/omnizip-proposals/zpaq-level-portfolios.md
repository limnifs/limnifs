# Proposal: ZPAQ — wire level → model portfolio

**Filed by:** LimniFS
**omnizip-rs crate:** `omnizip-zpaq`
**Severity:** feature gap (level parameter is accepted but ignored)

## Problem

`omnizip-zpaq` exposes `Codec::compress(plaintext, level)` and
accepts `level ∈ 0..=9`. The implementation ignores `level` and
uses one fixed model portfolio (today: order-0, order-1, order-2,
order-3, match, run-length — 6 models). Quoting
`omnizip-zpaq/src/lib.rs`:

> Phase 2 uses a single fixed model portfolio for all levels; the
> level parameter is currently accepted but does not switch models.

LimniFS exposes ZPAQ in its `max-ratio` profile, where the user
expects the highest-effort setting. Today there is no observable
difference between level 1 and level 9.

## Proposed design

Two portfolios, dispatched by level:

| Level | Portfolio | Memory | Encode speed | Use case |
|---|---|---|---:|---|
| 0..=2 | `Fast`: order-0, order-1, match | O(1) + O(256) | fast | inline metadata, tiny files |
| 3..=5 | `Default`: + order-2, run-length | + O(64K) | medium | general text/source |
| 6..=9 | `Best`: + order-3 | + O(16M) HashMap | slow | archival text (Enwik8) |

The portfolios nest: `Best` ⊃ `Default` ⊃ `Fast`. Each is a
constfied list of model indices into a static `ALL_MODELS` array.

```rust
const PORTFOLIO_FAST: &[ModelKind] = &[ModelKind::Order0, ModelKind::Order1, ModelKind::Match];
const PORTFOLIO_DEFAULT: &[ModelKind] = &[ModelKind::Order0, ModelKind::Order1, ModelKind::Order2, ModelKind::Match, ModelKind::RunLength];
const PORTFOLIO_BEST: &[ModelKind] = &[ModelKind::Order0, ModelKind::Order1, ModelKind::Order2, ModelKind::Order3, ModelKind::Match, ModelKind::RunLength];
```

`MultiModel::new(portfolio)` selects which models to instantiate.
The mixer's `NUM_MODELS` becomes `portfolio.len()`.

## Backwards compatibility

The wire container's header already carries a "model configuration
id" byte. Reserve:

| id | portfolio |
|---|---|
| 0 | Phase-1 (4 models, no order-3) — legacy |
| 1 | Phase-2 default (5 models, run-length) |
| 2 | Phase-2 best (6 models, + order-3) |
| 3 | Fast (3 models) — new |
| 4 | Default — new |
| 5 | Best — new |

Decompress reads the id and reconstructs the same portfolio. Old
containers (ids 0..=2) keep working.

## Acceptance

- [ ] `ZpaqCodec::compress(plaintext, level)` produces different
      output for level=1 vs level=9 on the same input.
- [ ] Round-trip succeeds for all (level, portfolio) combinations.
- [ ] Decode of legacy containers (ids 0..=2) still works unchanged.
- [ ] Benchmark on Calgary `book1`: level 9 ≥ 5% smaller than
      level 1.

## Why LimniFS cares

LimniFS exposes ZPAQ as a tournament candidate for text in the
`max-ratio` profile. Without level control, we can't ask for "best"
explicitly. With this proposal, LimniFS maps
`profile.codec_tunables.zpaq_level` (a new field) directly to the
codec level.

## Effort estimate

1–2 days. The model structs already exist; this is portfolio
selection + container id plumbing.

## Related

- `omnizip-rs TODO.complete/80-zpaq-more-models.md` — the run-length
  + order-3 additions are the prerequisites; both are landed.
- `omnizip-rs TODO.complete/87-differential-harness.md` — once the
  harness lands, the level/portfolio combos become regression-test
  fixtures.
