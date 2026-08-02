# Proposal: omnizip 0.4 follow-ups (ZSTD, LZMA, FLAC)

omnizip 0.4 shipped FSST, Rice++, FLAC skeleton, and the Codec trait
extension. Three things still block LimniFS from hitting parity with
libzstd/liblzma/libFLAC on the workloads where it currently loses.
This proposal documents each with reproduction steps and acceptance
criteria so the omnizip team can pick them up.

## P0 — ZSTD Phase C: FSE state-transition bug

**User report:** "ZSTD Phase C FSE bitstream — has a state-transition
bug causing offset corruption for some patterns. Round-trip safety
check falls back to Raw blocks. This is the critical path to hitting
≤12% ratio on source code at L6."

### Symptom

`omnizip_zstd::encode_frame(input, ZstdLevel::Default)` produces a
valid ZSTD frame that round-trips, but for many inputs the encoder
detects that its own output would not decompress correctly and
**silently falls back to emitting Raw blocks**. The output is then
slightly larger than the input — no compression happens.

Reproduced on 90 KB of `"the quick brown fox jumps over the lazy dog. "`
repeated: all 5 levels (`Fastest`, `Fast`, `Default`, `Better`,
`Best`) produce 90 017 bytes (100.02%).

### What's broken

The FSE (Finite State Entropy) bitstream encoder writes state
transitions in the wrong order for some offset-code distributions.
When the decoder runs, it reads state bits before extra bits (or
vice versa) for those distributions and computes a different offset
than the encoder intended. The encoder's round-trip safety check
catches the mismatch and falls back.

The bit-position alignment bug is the same shape as the decoder
side already fixed (commit `bb46611` "fix FSE decode order — extra
bits before state transitions"). The encoder needs the inverse fix.

### Acceptance criteria

1. `encode_frame(enwik8, ZstdLevel::Default)` produces output ≤
   36.5 MB (libzstd L6 reference is 36.47 MB).
2. `encode_frame(enwik8, ZstdLevel::Fastest)` produces output ≤
   40 MB (libzstd L1 reference is 39.7 MB).
3. No "safety check fell back to Raw" warnings on any input in the
   differential test corpus.
4. LimniFS php-create benchmark drops from 1.1 s (Brotli q5) to
   ≤ 0.6 s with ZSTD L6 routing.

### Reproduction

```bash
cd /path/to/omnizip-rs
cargo test -p omnizip-zstd --release encode_beats_raw_on_repetitive
```

Add this test:

```rust
#[test]
fn encode_beats_raw_on_repetitive() {
    let input = b"the quick brown fox jumps over the lazy dog. ".repeat(2000);
    for level in &[ZstdLevel::Fastest, ZstdLevel::Fast, ZstdLevel::Default,
                   ZstdLevel::Better, ZstdLevel::Best] {
        let c = encode_frame(&input, *level).unwrap();
        assert!(c.len() < input.len(),
            "level {level}: expected compression, got {} bytes (input {})",
            c.len(), input.len());
    }
}
```

Currently fails on all 5 levels.

## P0 — ZSTD Huffman literals: encoder not wired

**User report:** "ZSTD Huffman literals — encoder exists but isn't
wired into the compressed block path (using Raw literals)."

### Symptom

Even after the FSE bug is fixed, ZSTD's compressed-block path will
emit literals as Raw (uncompressed) bytes. Literals are typically
40-60% of source-code bytes; compressing them with Huffman would
roughly double the overall ratio gain.

### What's broken

The Huffman encoder exists in `omnizip-zstd/src/huffman/encoder.rs`
(function `encode_literals`). It is not called by the compressed-
block path. The block encoder hard-codes `literals_block_type =
Raw` regardless of whether Huffman would help.

### Acceptance criteria

1. `encode_frame` chooses Huffman literals when the Huffman-encoded
   literal section is smaller than Raw (current libzstd behaviour).
2. Differential test against libzstd output on enwik8 produces
   byte-identical literal sections (or close — within 1% of size).
3. Combined with the FSE fix above, ratio on enwik8 at L6 ≤ 36.5 MB.

### Reproduction

Add a unit test in `omnizip-zstd/src/literals/mod.rs`:

```rust
#[test]
fn huffman_literals_chosen_when_smaller() {
    // Literals with skewed distribution (lots of 'e', 't', 'a')
    // where Huffman crushes Raw.
    let literals: Vec<u8> = b"eeeettttaaaaeeee".repeat(1024);
    let block = encode_literals_section(&literals);
    assert!(block.len() < literals.len(),
        "Huffman should be chosen; got {} bytes vs {} raw",
        block.len(), literals.len());
}
```

Currently fails because the encoder always emits Raw.

## P1 — LZMA Phase C: match finder not wired

**User report:** "LZMA Phase C — match finder exists
(`encoder/match_finder.rs`), needs wiring into the LZMA2 chunk
encoder."

### Symptom

`omnizip_lzma::xz_compress(input)` produces output larger than the
input (Phase B literal-only). The match finder code exists but
`Lzma1Encoder::encode` ignores its output and emits one literal
packet per input byte.

### Acceptance criteria

1. `xz_compress(enwik8)` produces output ≤ 27 MB (liblzma L6
   reference is 26.9 MB).
2. `xz_compress(enwik8, LzmaLevel::BEST)` produces output ≤ 23 MB
   (liblzma L9 reference is 22.9 MB).
3. Round-trips through the existing `xz_decompress`.
4. Differential test against `xz -6` produces byte-identical output
   on the omnizip test corpus (or within 2% if exact-match isn't
   feasible due to encoder heuristics).

### Reproduction

```rust
#[test]
fn xz_actually_compresses() {
    let input = b"the quick brown fox jumps over the lazy dog. ".repeat(1000);
    let c = omnizip_lzma::xz_compress(&input).unwrap();
    assert!(c.len() < input.len(),
        "xz encode should beat raw, got {} vs {}", c.len(), input.len());
}
```

Currently fails.

## P2 — FLAC: full LPC + Rice codec

**User report:** "FLAC — only header parsers + raw PCM container.
Full LPC + Rice residual codec is a large port from libFLAC (~20K LOC)."

### Symptom

`omnizip_flac::compress(input, params)` produces a valid FLAC frame
header but the body is raw PCM bytes — no linear prediction, no
Rice coding. The output is larger than the input. Round-trips
correctly through `omnizip_flac::decompress`.

### What's missing

1. **Linear prediction**: compute LPC coefficients from the sample
   data, encode as fixed-order or quantised-order predictors.
2. **Residual coding**: code the prediction residuals with Rice
   coding (variable order per block).
3. **Block splitting**: partition the input into blocks of fixed
   sample count (4096 typical), each with its own LPC + residual
   parameters.
4. **Frame header**: emit the per-block parameters in the FLAC
   frame header.

Reference: libFLAC (`git:libFLAC/`), BSD-3-Clause license. ~20K LOC.

### Acceptance criteria

1. `compress(enwik_pcm_16bit_44k_mono, PcmParams{...})` produces
   output ≤ 18% of input (libFLAC reference is ~17%).
2. Round-trips through the existing decoder.
3. Differential test against libFLAC output produces byte-identical
   PCM (frame layouts may differ; decoded audio must match).

### Why this is P2 not P0

LimniFS doesn't currently ship a WAV/AIFF benchmark dataset — the
ratio win is locked behind both omnizip-flac's full encoder AND
LimniFS adding audio to the bench. FSST and Rice++ already deliver
measurable wins today; FLAC can wait.

## Summary of asks

| Priority | Item | Acceptance | Estimated effort |
|---|---|---|---|
| **P0** | ZSTD FSE state-transition bug fix | enwik8 L6 ≤ 36.5 MB | 1–2 days (similar shape to decoder fix) |
| **P0** | ZSTD Huffman literals wiring | chosen when smaller than Raw | 1 day |
| **P1** | LZMA match finder wiring | enwik8 L6 ≤ 27 MB | 1 week (LZMA encoder is intricate) |
| **P2** | FLAC full LPC + Rice codec | PCM ratio ≤ 18% | 2–3 months (20K LOC port) |

## LimniFS-side status

LimniFS is fully wired for all four pieces — when omnizip ships a
fix, integration is "bump dep version, re-bench". No architectural
changes needed.

| omnizip fix | LimniFS work |
|---|---|
| ZSTD FSE + Huffman | Switch `best_compressible_codec()` from Brotli back to ZSTD L6; ~5 LOC. |
| LZMA match finder | Update `xz.rs` to pass `LzmaLevel::Default` instead of ignoring level; ~5 LOC. |
| FLAC full codec | Replace `FlacCodec` stub with `omnizip-flac::FlacCodec` wrapper; ~30 LOC. |

## References

- `docs/omnizip-rs-proposal.md` — original omnizip asks (this doc supersedes the codec-id reservations; they're now done)
- `docs/omnizip-vs-limnifs-boundary.md` — codec vs categorizer boundary
- `docs/dwarfs-multicodec-investigation.md` — DwarFS source citations
- omnizip BUGREPORT: `BUGREPORT-zstd-0.1.0.md` at omnizip-rs root
