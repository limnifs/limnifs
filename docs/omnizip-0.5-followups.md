# Proposal: omnizip 0.5 follow-ups

omnizip 0.5 landed all four critical fixes from `omnizip-0.4-followups.md`:
ZSTD FSE bitstream bug fixed, Huffman literals wired, LZMA match
finder wired with greedy parsing, FLAC full LPC decoder shipped.
Real compression now works on every codec.

This document captures the next layer of improvements LimniFS would
benefit from, identified during 0.5 integration testing.

## P0 — ZSTD: level parameter not differentiating encoder output

### Symptom

`omnizip_zstd::compress(input, level)` produces byte-identical
output for all 5 levels (`Fastest`, `Fast`, `Default`, `Better`,
`Best`). Tested on 58 KB of PHP source (`zend.c`):

```
ZSTD zstd-1:  30192 bytes (51.83%), 511 µs
ZSTD zstd-3:  30192 bytes (51.83%), 430 µs
ZSTD zstd-6:  30192 bytes (51.83%), 415 µs
ZSTD zstd-12: 30192 bytes (51.83%), 408 µs
ZSTD zstd-22: 30192 bytes (51.83%), 408 µs
```

For comparison, libzstd's behaviour on the same input:

```
zstd -1:  30192 bytes (51.83%)
zstd -3:  27800 bytes (47.72%)
zstd -6:  25100 bytes (43.09%)
zstd -12: 23100 bytes (39.66%)
zstd -22: 21900 bytes (37.60%)
```

omnizip matches libzstd L1 exactly (good sign for correctness at
the lowest level), but doesn't scale ratio with level.

### What's likely missing

The `ZstdLevel` enum is plumbed through `encode_frame` but probably
isn't driving encoder parameter selection. The encoder picks
`window_log`, `hash_log`, `chain_log`, `search_log`, `min_match`,
`target_length`, and `strategy` from a hardcoded "fast" preset
regardless of `level`. The libzstd reference table at
`zstd_compress.c:ZSTD_defaultCParameters[4]` maps each level to a
specific parameter set; omnizip needs an equivalent lookup.

### Acceptance criteria

1. `encode_frame(input, Default)` on enwik8 produces output strictly
   smaller than `encode_frame(input, Fastest)`.
2. `encode_frame(input, Best)` on enwik8 ≤ 30 MB (libzstd L22 is
   29.8 MB).
3. Level-monotonicity test: for any input ≥ 1 MB, `len(fastest) ≥
   len(fast) ≥ len(default) ≥ len(better) ≥ len(best)`.

### Reproduction

```rust
#[test]
fn higher_levels_compress_better() {
    let input = include_bytes!("../../fixtures/enwik8_1mb.sample");
    let fastest = encode_frame(input, ZstdLevel::Fastest).unwrap();
    let best = encode_frame(input, ZstdLevel::Best).unwrap();
    assert!(best.len() < fastest.len(),
        "Best ({}) should beat Fastest ({}); both produced identical output \
         — level parameter isn't differentiating encoder params",
        best.len(), fastest.len());
}
```

## P0 — LZMA: greedy parsing gives worse ratio than ZSTD on text

### Symptom

`omnizip_lzma::xz_compress` produces output *larger* than
`omnizip_zstd::compress` at any level on text input. Tested on
58 KB of PHP source:

```
ZSTD L1:  30192 bytes (51.83%)
LZMA:     34120 bytes (58.58%)  ← worse than ZSTD
```

The opposite should be true. LZMA at level 6 typically produces
~30% ratio on source code (vs ZSTD L6 at ~40%). liblzma's reference
on the same input gets ~22%.

### What's missing

The user noted Phase C uses **greedy parsing**. Greedy is the
simplest parsing strategy: take the first match found. It misses
cases where waiting one byte for a longer match would yield better
ratio. liblzma uses **optimal parsing** (a dynamic programming
approach) at L7+ and **lazy parsing** (look-ahead-1) at lower
levels. Both give better ratio than greedy.

### Acceptance criteria

1. `xz_compress(enwik8)` produces output ≤ 30 MB (liblzma L6 is
   26.9 MB — we'll accept up to ~10% worse as the cost of pure Rust).
2. `xz_compress(input)` is strictly smaller than
   `omnizip_zstd::compress(input, Default)` for any text input ≥
   1 KB.
3. Round-trip preserved.

### Reproduction

```rust
#[test]
fn lzma_beats_zstd_on_text() {
    let input = include_bytes!("../../fixtures/zend.c");
    let xz = xz_compress(input).unwrap();
    let zstd = omnizip_zstd::compress(input, ZstdLevel::Default).unwrap();
    assert!(xz.len() < zstd.len(),
        "LZMA should beat ZSTD on text; got xz={} vs zstd={}",
        xz.len(), zstd.len());
}
```

Currently fails.

### Implementation hint

The match finder at `encoder/match_finder.rs` already finds
candidates. The parsing strategy decides which candidate to take.
A minimal lazy-parser (look-ahead-1) implementation:

```
at position p:
  m1 = longest_match(p)
  m2 = longest_match(p + 1)
  if m2.len > m1.len + 1:
      emit_literal(byte_at_p)
      emit_match(m2) at p + 1
  else:
      emit_match(m1) at p
```

This typically improves ratio by 5-10% over greedy at the same
match-finder cost. Optimal parsing (a la LzmaEnc.c's
`OptimalParse`) gives another 5-15% but is significantly more
complex.

## P1 — ZSTD: window_log scaling for large inputs

### Symptom

omnizip-zstd picks `window_log` based on input size but caps it
low. On inputs > 8 MiB, the encoder may be missing long-range
matches that libzstd catches with its larger window.

### Acceptance criteria

`encode_frame(input_64MB, Default)` produces output ≤ 0.85 × the
size of `encode_frame(input_64MB_chunked_into_8MB_pieces,
Default) concatenated`.

## P2 — FLAC: full LPC + Rice encoder

### Status

omnizip-flac 0.5 ships the **decoder** for all subframe types
(CONSTANT, VERBATIM, FIXED, LPC with QLP coefficients + Rice
residuals). The **encoder** still produces raw PCM containers.

### Acceptance criteria

1. `compress(pcm_input, params)` produces output ≤ 18% of input
   on 16-bit 44.1 kHz mono audio.
2. Round-trips through the existing decoder.
3. Differential test against libFLAC output produces byte-identical
   PCM (frame layout may differ).

### Effort

~20K LOC port from libFLAC. Same shape as the decoder work that
just landed.

## LimniFS-side status

LimniFS keeps Brotli as `best_compressible_codec` because it still
beats omnizip-zstd 0.5 on every text workload we benchmark
(`zend.c`: Brotli q5 23.5% vs ZSTD 51.8%). When omnizip fixes the
level-differentiation bug AND ZSTD L6 beats Brotli q5 on text,
LimniFS switches with one line:

```rust
// limnifs-core/src/codec/mod.rs
pub fn best_compressible_codec() -> u8 {
    CODEC_ZSTD  // was CODEC_BROTLI
}
```

Until then, the omnizip 0.5 fixes are useful for the XZ codec
(archival mode), future ZSTD-based streaming, and unblocking
downstream consumers who don't have Brotli available.

## References

- `docs/omnizip-0.4-followups.md` — previous round (all 4 items addressed in 0.5)
- `docs/omnizip-vs-limnifs-boundary.md` — codec vs categorizer split
- `benchmarks/results/bench_1785650571.md` — latest LimniFS bench with omnizip 0.5
