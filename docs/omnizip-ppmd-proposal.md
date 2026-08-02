# Proposal: PPMd clean-room implementation for omnizip-rs

## Summary

Implement PPMd (Prediction by Partial Matching with defer) as a
pure-Rust codec crate `omnizip-ppmd`. Derived from the published
academic literature (Cleary & Witten 1984; Shkarin DCC 2001),
NOT from the LGPL reference implementation in 7-Zip.

## Why PPMd

PPMd uses **context-tree weighting**: it builds a suffix tree of
recent contexts and predicts the next symbol by counting how often
each symbol followed similar contexts in the past. The probability
estimates adapt online as more data is processed.

**PPMd's niche**: best-in-class ratio on **natural language text**
(English, prose, documentation). Typically beats Brotli q11 by
5-15% on text-heavy workloads. Used in RAR and 7-Zip for this
reason.

Where PPMd loses: binary data, media, already-compressed content.
PPMd's adaptive context tree has high per-byte overhead that
doesn't pay off on low-redundancy input.

## Clean-room basis

| Source | License | Used for |
|---|---|---|
| Cleary & Witten 1984 ("Data Compression Using Adaptive Coding") | Academic paper (fair use) | PPM* algorithm description, context-tree structure |
| Shkarin DCC 2001 ("PPM: one step to practicality") | Academic paper (fair use) | PPMd escape mechanism, probability update, model truncation |
| 7-Zip `PPMd` C source | LGPL-2.1 | **NOT READ by the implementer.** Used only for differential testing (run as black box). |
| `pyppmd` Python impl | Apache-2.0 | API reference; may read for interface design (Apache-compatible) |

The implementer reads the two academic papers. A separate tester
runs the 7-Zip binary as a black box for differential comparison.

## Architecture

```
omnizip-ppmd/
├── Cargo.toml
├── src/
│   ├── lib.rs                 # Public API + Codec trait impl
│   ├── context_tree.rs        # PPM context tree (trie with counts)
│   ├── probability.rs         # Probability estimation + update
│   ├── escape.rs              # Escape mechanism (PPM+ style)
│   ├── range_coder.rs         # Binary arithmetic range coder
│   ├── model.rs               # Model order selection + truncation
│   └── codec.rs               # Codec trait wrapper
└── tests/
    ├── round_trip.rs
    ├── differential.rs        # vs 7-Zip PPMd (black-box)
    └── text_ratio.rs          # Verify PPMd beats Brotli on text
```

## Algorithm summary (from the papers)

### Context tree

Maintain a trie of depth `O` (model order, typically 4-16).
Each node stores symbol occurrence counts. To predict symbol `s`
at position `p`, walk the trie following the last `O` symbols
before `p`. At each depth, look up the counts for the children.

### Probability estimation

At the deepest context (order `O`), probability of symbol `s` is:

```
P(s) = count[s] / total_count
```

If `s` has zero count (unseen symbol), invoke the **escape
mechanism**: emit an escape token, fall back to order `O-1`, and
retry. Continue falling back until the symbol is found or order-0
is reached. At order-0, if still not found, use a uniform prior.

### Shkarin's improvements (PPMd variant)

1. **Symbol exclusion**: when escaping from order `k` to order
   `k-1`, exclude symbols that had nonzero counts at order `k`.
   This narrows the search and improves ratio.

2. **Probability inheritance**: when a new context node is
   created at order `k`, inherit counts from its parent at
   order `k-1`. This bootstraps cold-start contexts.

3. **Count rescaling**: when any count exceeds a threshold
   (typically 255), halve all counts in the node. This adapts
   to distribution drift.

### Range coder

Binary arithmetic coder. Encodes one bit at a time using the
model's probability estimate. The ZPAQ proposal also uses a
range coder — the implementations could share code via a
`omnizip-arith` shared crate.

## Implementation plan (phased)

### Phase 1: Core PPM with escape (~1000 LOC)

- `context_tree.rs`: trie with symbol counts. ~400 LOC.
- `probability.rs`: count-based estimation + rescaling. ~200 LOC.
- `escape.rs`: PPM+ escape mechanism. ~200 LOC.
- `range_coder.rs`: binary arithmetic coder. ~200 LOC.
- **Acceptance**: round-trips. Order-4 model. Ratio ~25% on
  enwik8 (comparable to LZMA-1).

### Phase 2: Shkarin improvements (~500 LOC)

- `model.rs`: symbol exclusion + probability inheritance + order
  truncation. ~500 LOC.
- **Acceptance**: ratio ≤ 20% on enwik8 (comparable to Brotli q5).

### Phase 3: Optimisation (~500 LOC)

- Sliding-window context tree pruning (avoid unbounded memory).
- SIMD count updates for common symbol distributions.
- **Acceptance**: ratio ≤ 18% on enwik8 at order 8. Beats
  Brotli q11. Encode speed ≥ 2 MB/s.

## API

```rust
pub struct PpmdConfig {
    pub max_order: u8,       // Context depth (4-16; default 6)
    pub mem_limit_mb: u8,    // Context tree memory cap (default 64)
}

pub fn compress(plaintext: &[u8], config: &PpmdConfig) -> Result<Vec<u8>, PpmdError>;
pub fn decompress(compressed: &[u8]) -> Result<Vec<u8>, PpmdError>;

pub struct PpmdCodec;
impl Codec for PpmdCodec { /* ... */ }
```

## Acceptance criteria

1. `compress(enwik8, order=8)` produces output ≤ 18 MB (7-Zip
   PPMd at order 8 gets ~17 MB; Brotli q11 gets ~19 MB).
2. `compress` on the Calgary Corpus text files produces output
   strictly smaller than Brotli q11 on at least 80% of files.
3. Round-trip: `decompress(compress(x, c)) == x`.
4. Determinism: same input + same config → byte-identical output
   across runs.
5. Memory: context tree stays within `mem_limit_mb` during
   encode and decode.
6. No `unsafe` code.

## Estimated effort

| Phase | LOC | Duration |
|---|---:|---|
| Phase 1 (core PPM) | 1000 | 1.5 weeks |
| Phase 2 (Shkarin improvements) | 500 | 1 week |
| Phase 3 (optimisation) | 500 | 1 week |
| Differential tests | 200 | 2 days |
| **Total** | **~2200** | **~3.5 weeks** |

## LimniFS integration

- New codec id **0x0C** = PPMd.
- Routing: PPMd for `Text` class when `--codec-map=archival`
  mode is active. Not used by default (too slow for real-time
  create; Brotli q5 is the default for text).

## References

- Cleary & Witten, "Data Compression Using Adaptive Coding and
  Partial String Matching", IEEE Trans. Comms. 1984.
- Shkarin, "PPM: one step to practicality", DCC 2001.
- 7-Zip PPMd source (LGPL, for differential testing only):
  https://github.com/ip7z/7zip/tree/main/CPP/7zip/Compress/PpmdZip.cpp
- `pyppmd` (Apache-2.0, API reference):
  https://github.com/dan200/pyppmd
