# Proposal: ZPAQ clean-room implementation for omnizip-rs

## Summary

Implement ZPAQ — the best-ratio general-purpose compression
algorithm in existence — as a pure-Rust codec crate
`omnizip-zpaq`. The implementation is derived from Matt Mahoney's
**public-domain** format specification (`zpaq.pdf` / `zpaq.txt`),
NOT from the GPL-3 reference C++ source. The reimplementation is
independently MIT/Apache-licensed.

## Why ZPAQ

ZPAQ uses **context mixing**: multiple independent models predict
the next bit's probability; a logistic mixer combines them; an
arithmetic coder encodes. The models include:

- Order-0 through order-2 context models
- Word model (detects ASCII word boundaries)
- Match model (finds long-range matches via LZP)
- Record model (detects fixed-width record boundaries)

This is structurally different from LZ/dictionary codecs (LZMA,
ZSTD, Brotli) and wins on data where repeated patterns span
large distances that LZ's sliding window misses.

**Typical ZPAQ ratio advantage over LZMA**: 10-20% smaller output
on text; 5-15% on binary. On the Silesia corpus, ZPAQ level 5
produces ~10% smaller output than LZMA level 9.

## Clean-room basis

| Source | License | Used for |
|---|---|---|
| `zpaq.pdf` / `zpaq.txt` (Mahoney) | **Public domain** | Format spec, VM ISA, model configs, container format |
| Mahoney DCC 2006/2009 papers | Academic (fair use) | Algorithm description, context-mixing theory |
| GPL-3 `zpaq` C++ source | GPL-3 | **NOT READ by the implementer**. Used only for differential testing (run as a black box, compare output). |

The implementer reads the public-domain spec. A separate tester
runs the GPL binary as a black box for differential comparison.
No GPL code enters the Rust source.

## Architecture

```
omnizip-zpaq/
├── Cargo.toml
├── src/
│   ├── lib.rs                 # Public API + Codec trait impl
│   ├── vm.rs                  # ZPAQ bytecode VM (predictor configs)
│   ├── models.rs              # Standard model configurations
│   ├── mixer.rs               # Logistic mixer (combines model outputs)
│   ├── arithmetic.rs          # Arithmetic/range coder
│   ├── container.rs           # ZPAQ archive container (blocks + segments)
│   └── codec.rs               # Codec trait wrapper
└── tests/
    ├── round_trip.rs          # Encode/decode identity tests
    ├── differential.rs        # vs `zpaq` CLI (black-box subprocess)
    └── determinism.rs         # Same input → same output across runs
```

## Implementation plan (phased)

### Phase 1: Arithmetic coder + single model (~800 LOC)

- `arithmetic.rs`: range coder with bit-level precision. ~300 LOC.
- `models.rs`: order-2 context model only (simplest useful model).
- `mixer.rs`: trivial single-model pass-through (no mixing yet).
- `container.rs`: minimal ZPAQ container (1 block, 1 segment).
- **Acceptance**: round-trips on enwik8. Ratio ~40% (worse than
  ZSTD but validates the arithmetic coder + container).

### Phase 2: Context mixer + multi-model (~1200 LOC)

- `mixer.rs`: logistic mixer with SSE (Secondary Symbol Estimation).
  Combines 2+ model outputs via weighted sigmoid. ~400 LOC.
- `models.rs`: add order-0, order-1, word, match models. ~500 LOC.
- `vm.rs`: ZPAQ bytecode VM for model configuration. ~300 LOC.
  The VM executes a simple stack-based ISA that configures context
  hashes, mixer weights, and probability lookup tables.
- **Acceptance**: ratio ≤ 20% on enwik8. Beats LZMA-1.

### Phase 3: Standard configs + optimisation (~1000 LOC)

- `models.rs`: port the standard ZPAQ configurations (levels 1-5)
  from the spec's published model bytecode. ~500 LOC.
- `container.rs`: full multi-block, multi-segment container with
  journaling support. ~300 LOC.
- Performance: SIMD-accelerated context hashing, branch-prediction-
  friendly mixer inner loop. ~200 LOC.
- **Acceptance**: ratio ≤ 15% on enwik8 at level 3. Beats LZMA-6.

## Determinism guarantees

ZPAQ IS deterministic when the model configuration is pinned. The
"non-determinism" sometimes attributed to ZPAQ is actually about
the reference encoder changing its default model tuning between
versions — not about the algorithm itself.

`omnizip-zpaq` will:
1. Pin the model configuration to a specific bytecode blob baked
   into the crate. Same input + same level → byte-identical output
   across runs, versions, and hosts.
2. Never use floating-point in the hot path (all probability
   arithmetic is integer-based). No SIMD-precision divergence.
3. Include a `determinism.rs` test that encodes the same input
   twice and asserts byte-equality.

This satisfies LimniFS's `DropId = BLAKE3(plaintext)` invariant:
the slab bytes must reproduce identically so the image Merkle root
is stable.

## API

```rust
pub enum ZpaqLevel {
    /// Fast: order-2 context only. ~50 MB/s encode.
    Fast,
    /// Default: multi-model with logistic mixing. ~5 MB/s encode.
    Default,
    /// Best: all standard models + SSE. ~1 MB/s encode.
    Best,
}

pub fn compress(plaintext: &[u8], level: ZpaqLevel) -> Result<Vec<u8>, ZpaqError>;
pub fn decompress(compressed: &[u8]) -> Result<Vec<u8>, ZpaqError>;

pub struct ZpaqCodec;
impl Codec for ZpaqCodec { /* ... */ }
```

## Acceptance criteria

1. `compress(enwik8, Default)` produces output ≤ 18 MB (reference
   ZPAQ level 3 is ~17 MB; LZMA-6 is ~27 MB).
2. `compress(enwik8, Best)` produces output ≤ 15 MB (reference
   ZPAQ level 5 is ~14 MB).
3. Round-trip: `decompress(compress(x, level)) == x` for any input.
4. Determinism: `compress(x, level) == compress(x, level)` across
   two invocations, two processes, two hosts (same arch).
5. Differential: `decompress(zpaq_cli_output)` succeeds (reads
   archives produced by the GPL reference binary).
6. No `unsafe` code. `#![forbid(unsafe_code)]`.

## Estimated effort

| Phase | LOC | Duration |
|---|---:|---|
| Phase 1 (arithmetic + single model) | 800 | 1 week |
| Phase 2 (mixer + multi-model + VM) | 1200 | 2 weeks |
| Phase 3 (standard configs + optimisation) | 1000 | 1 week |
| Differential tests + CI integration | 200 | 2 days |
| **Total** | **~3200** | **~4 weeks** |

Comparable to the LZMA port that already shipped (~3000 LOC).

## LimniFS integration

When omnizip-zpaq ships:
- New codec id **0x0B** = ZPAQ.
- LimniFS wires a `ZpaqCodec` wrapper (~30 LOC).
- Routing: ZPAQ is the "archival" codec for
  `--codec-map=archival` mode. Not used by default (too slow
  for real-time create). Used when users explicitly request
  maximum ratio.

## Why omnizip should do this

1. **Best ratio in existence.** No other lossless codec
   consistently beats ZPAQ on diverse data.
2. **The spec is public domain.** Zero legal ambiguity for
   clean-room reimplementation.
3. **Fills a gap no current omnizip codec covers.** LZMA is the
   current best at ~22% on enwik8; ZPAQ hits ~14%. The 8-point
   gap is the single biggest available ratio win in the codec
   ecosystem.
4. **Context mixing is architecturally different** from LZ/entropy
   approaches. It's a complementary codec, not a competitor to
   existing ones.

## References

- Spec: http://mattmahoney.net/zpaq/zpaq.pdf (public domain)
- Mahoney, "PAQ8" / "ZPAQ" papers at DCC 2006, 2009
- Reference impl (GPL, for differential testing only):
  https://github.com/zpaq/zpaq
- Silesia benchmark results:
  http://mattmahoney.net/dc/silesia.html
