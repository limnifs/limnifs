# Proposal: GLZA clean-room implementation for omnizip-rs

## Summary

Implement GLZA (Grammar-based LZ Compression) as a pure-Rust codec
crate `omnizip-glza`. Derived from Gregory Smith's published
format specification and academic description, NOT from the GPL-3
reference C source.

## Why GLZA

GLZA builds a **context-free grammar** describing the input. It
replaces repeated substrings with grammar rules, then encodes the
rule stream with an entropy coder. This is fundamentally different
from both LZ (dictionary + sliding window) and BWT (block sorting)
approaches.

**GLZA's niche**: inputs with **hierarchical repetition** —
repeated structures that contain other repeated structures. Examples:

- **DNA/genomics**: repetitive elements, tandem repeats, transposons.
  GLZA typically achieves 20-30% ratio on human DNA where LZMA
  gets 35-40%.
- **Log files**: repeated timestamp patterns, log levels, message
  templates with nested structure.
- **XML/HTML**: tag structures with repeated attributes and nested
  elements.
- **JSON APIs**: repeated key names + nested object structures.

Where GLZA loses: random/binary data with no structural repetition.
It falls back to near-store ratios. It's also slower than LZMA
(grammar construction is O(n log n) with a large constant).

## Clean-room basis

| Source | License | Used for |
|---|---|---|
| Smith's `GLZA_format.md` (in repo's `/doc`) | Published spec | Wire format, grammar encoding, entropy coding |
| Smith's academic paper / README | Published description | Algorithm: grammar construction, rule dedup, suffix sorting |
| GPL-3 `GLZA` C source | GPL-3 | **NOT READ by the implementer.** Used only for differential testing. |

The implementer reads the format spec and algorithm description.
A separate tester runs the GPL binary as a black box for comparison.

## Architecture

```
omnizip-glza/
├── Cargo.toml
├── src/
│   ├── lib.rs                 # Public API + Codec trait impl
│   ├── grammar.rs             # Grammar construction (rule extraction)
│   ├── suffix_array.rs        # Suffix array for substring discovery
│   ├── rules.rs               # Rule dedup + numbering
│   ├── encode.rs              # Grammar → entropy-coded stream
│   ├── decode.rs              # Entropy stream → grammar → expand
│   └── codec.rs               # Codec trait wrapper
└── tests/
    ├── round_trip.rs
    ├── differential.rs        # vs GLZA CLI (black-box)
    └── dna_ratio.rs           # Verify GLZA beats LZMA on DNA
```

## Algorithm summary

### Grammar construction

1. Build a **suffix array** of the input. This gives sorted
   positions of all suffixes, enabling O(n) identification of the
   longest repeated substrings.

2. Iteratively extract the most profitable repeated substring and
   replace all its occurrences with a grammar rule reference:
   ```
   Rule_i → substring
   ```
   "Profitable" = the rule saves more bytes (occurrences × length
   minus rule definition overhead) than it costs.

3. Repeat until no substring replacement is profitable.

4. The grammar is now a DAG (directed acyclic graph) of rules,
   where each rule body may reference other rules.

### Encoding the grammar

1. Topologically sort the rule DAG.
2. Emit each rule body as a sequence of symbols (terminals +
   rule references).
3. Apply entropy coding (arithmetic or Huffman) to the symbol
   stream. Symbols that are rule references get shorter codes
   than raw bytes.

### Decoding

1. Decode the entropy-coded symbol stream.
2. Reconstruct the rule table.
3. Expand the start symbol recursively.

## Implementation plan (phased)

### Phase 1: Suffix array + grammar construction (~1500 LOC)

- `suffix_array.rs`: SA-IS algorithm (linear-time suffix array
  construction). ~600 LOC. Well-described in Nong et al. 2009.
- `grammar.rs`: greedy rule extraction with profit estimation. ~500 LOC.
- `rules.rs`: rule numbering + dedup. ~200 LOC.
- **Acceptance**: builds a grammar from input; round-trips with
  naive Huffman on the symbol stream.

### Phase 2: Entropy-coded grammar stream (~800 LOC)

- `encode.rs`: grammar → symbol stream → arithmetic/Huffman coding.
  ~500 LOC.
- `decode.rs`: reverse. ~300 LOC.
- **Acceptance**: round-trips through the wire format. Ratio ≤ 30%
  on a DNA test corpus.

### Phase 3: Optimisation (~700 LOC)

- Grammar pruning (remove unprofitable rules after construction).
- Parallel suffix array construction.
- Memory-bounded grammar (cap rule count; fall back to LZ for
  sections where grammar doesn't help).
- **Acceptance**: ratio ≤ 25% on DNA; encode speed ≥ 1 MB/s.

## API

```rust
pub struct GlzaConfig {
    pub max_rules: u32,       // Cap on grammar size (default 1M)
    pub min_rule_len: u8,     // Minimum substring length to extract (default 8)
}

pub fn compress(plaintext: &[u8], config: &GlzaConfig) -> Result<Vec<u8>, GlzaError>;
pub fn decompress(compressed: &[u8]) -> Result<Vec<u8>, GlzaError>;

pub struct GlzaCodec;
impl Codec for GlzaCodec { /* ... */ }
```

## Acceptance criteria

1. `compress(human_dna_sample)` produces output ≤ 30% of input
   (reference GLZA gets ~28%; LZMA-9 gets ~38%).
2. `compress(xml_sample)` produces output ≤ 12% of input
   (reference GLZA gets ~10%).
3. Round-trip preserved.
4. Determinism: same input + config → byte-identical output.
5. Memory: grammar stays within `max_rules` during construction.
6. No `unsafe` code.

## Estimated effort

| Phase | LOC | Duration |
|---|---:|---|
| Phase 1 (suffix array + grammar) | 1500 | 2 weeks |
| Phase 2 (entropy coding) | 800 | 1 week |
| Phase 3 (optimisation) | 700 | 1 week |
| Differential tests | 200 | 2 days |
| **Total** | **~3200** | **~4 weeks** |

## LimniFS integration

- New codec id **0x0D** = GLZA.
- Routing: GLZA for content classes where hierarchical repetition
  is detected (DNA files via `.fasta`/`.fastq` extension; XML via
  magic byte + extension). Not used by default.

## When GLZA is NOT worth it

GLZA's grammar construction is expensive (suffix array + iterative
extraction). On inputs with no hierarchical structure (source code
without repeated includes, random data, already-compressed files),
GLZA produces output larger than LZMA with 10× the encode time.
The LimniFS categorizer must gate GLZA behind a structural-
repetition heuristic.

## References

- Smith's GLZA repo (format spec in `/doc`, GPL source for
  differential testing only): https://github.com/dbandstra/GLZA
- Nong et al., "Linear Suffix Array Construction by Almost Pure
  Induced-Sorting", DCC 2009 (SA-IS algorithm for suffix arrays).
- Nevill-Manning & Witten, "Identifying Hierarchical Structure in
  Sequences: A linear-time algorithm", J. AI Research 1997
  (grammar inference background).
- DNA compression survey:
  https://academic.oup.com/bioinformatics/article/36/19/4853/5871575
