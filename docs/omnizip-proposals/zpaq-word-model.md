# Proposal: ZPAQ — word-level model with safe initialisation

**Filed by:** LimniFS
**omnizip-rs crate:** `omnizip-zpaq`
**Severity:** feature gap (TODO 80 word-level model deferred)

## Problem

`omnizip-rs TODO.complete/80-zpaq-more-models.md` marks the
word-level model as **deferred** with this rationale:

> A word-level model needs careful weight init to avoid regressing
> on short inputs (kilobytes where adaptation hasn't converged). The
> cmix-style "1000+ models" approach requires megabytes of input
> before paying off — LimniFS's typical payload is smaller.

LimniFS agrees with the deferral rationale for *unbounded* adaptation,
but a **bounded** word model is feasible today and would close most
of the natural-language gap. This proposal specifies the
initialisation that makes the model safe to ship.

## Proposed design

### Model: `WordModel`

Tokenises the input stream into ASCII alphanumeric runs (regex:
`[A-Za-z0-9_]+`, ≥ 3 bytes). Maintains two counters:

- `word_freq: HashMap<Vec<u8>, u32>` — frequency of the current
  word's bytes 0..N.
- `next_byte_freq: HashMap<(Vec<u8>, u8), u32>` — given current
  word prefix, frequency of each following byte.

Cap both maps at 65_536 entries; LRU-evict on overflow.

### Prediction

When the current byte position is inside a word, `WordModel::predict`
returns the next-byte distribution from `next_byte_freq`. Outside a
word (whitespace, punctuation), returns uniform.

### Mixer weight initialisation

The TODO's concern is that the mixer's adaptive logistic weights
haven't converged on short inputs. Solve this with a **prior**:

- `WordModel` starts with mixer weight = 0 (no influence).
- After processing `WARMUP = 16_384` bytes, the mixer has seen
  enough signal to assign WordModel a non-zero weight.
- Before warmup completes, WordModel contributes probability but
  the mixer's stretched sum isn't dominated by it.

Concretely, the model's output is gated:

```rust
fn predict(&self, ctx: &ModelContext) -> u16 {
    if ctx.bytes_processed < WARMUP || !ctx.in_word {
        return UNIFORM;  // 32768
    }
    // ... lookup next_byte_freq
}
```

This makes the model **zero-cost** until warmup, then **monotone
non-harmful** after warmup (any non-zero mixer weight only helps).

### Memory budget

`WordModel` declares `8 MiB` ceiling (two HashMaps × 64K entries ×
~64 bytes/entry). Within LimniFS's `max-ratio` profile budget.

## Acceptance

- [ ] `WordModel` added to `MultiModel` behind the `Best` portfolio
      (see [zpaq-level-portfolios.md](./zpaq-level-portfolios.md)).
- [ ] On Calgary `paper1` (53 KB), the `Best` portfolio is at least
      as good as today's 6-model mix; no regression on short inputs.
- [ ] On Enwik8 (100 MB), `Best` is ≥ 3% smaller than today's mix.
- [ ] Deterministic snapshot test (no `rng`).
- [ ] Memory stays under 8 MiB; LRU eviction verified.

## Why LimniFS cares

Natural-language corpora (maildirs, documentation trees, Wikipedia
dumps) are a real workload for archival images. LimniFS's
`max-ratio` profile would benefit measurably.

## Effort estimate

3 days:
- 1 day: model + tokeniser.
- 1 day: mixer wiring + warmup gate.
- 1 day: differential tests + benchmark.

## Related

- Completes omnizip-rs TODO 80.
- Depends on [zpaq-level-portfolios.md](./zpaq-level-portfolios.md)
  for the `Best` portfolio id.
