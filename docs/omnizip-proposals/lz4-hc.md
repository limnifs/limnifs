# Proposal: LZ4 HC — wire the real high-compression match finder

**Filed by:** LimniFS
**omnizip-rs crate:** `omnizip-lz4`
**Severity:** correctness (the codec claims HC but produces fast-LZ4 output)

## Problem

`omnizip-lz4 0.13.1` ships `Lz4HcCodec` whose `compress` body is byte-identical to `Lz4FastCodec::compress` — both call `lz4_flex::compress_prepend_size(plaintext)`. The HC match finder is never invoked. As a result, `Lz4HcCodec` produces the same output as `Lz4FastCodec` for every input.

Direct evidence (LimniFS verification, 2026-08-03):

```rust
use omnizip_codecs::{Codec, CompressionLevel};
let data: Vec<u8> = (0..50_000).map(|i| (i % 251) as u8).collect();
let fast = omnizip_lz4::Lz4FastCodec.compress(&data, CompressionLevel::default()).unwrap();
let hc = omnizip_lz4::Lz4HcCodec.compress(&data, CompressionLevel::default()).unwrap();
println!("orig={}, fast={}, hc={}", data.len(), fast.len(), hc.len());
// → orig=50000, fast=461, hc=461
```

The doc comment on `Lz4HcCodec` claims:

> LZ4 high-compression codec. Wraps `lz4_flex::compress_prepend_size`
> with the HC match finder.

…which is misleading: `compress_prepend_size` is the fast path. The
HC path in `lz4_flex` is `compress_hc_prepend_size` (gated behind
the `lz4` crate's `compress-hc` feature in older versions; available
unconditionally in `lz4_flex` ≥ 0.10).

## Proposed fix

Single-line change in `omnizip-lz4/src/lib.rs`:

```rust
impl Codec for Lz4HcCodec {
    fn compress(&self, plaintext: &[u8], _level: CompressionLevel) -> Result<Vec<u8>, OmnizipError> {
        Ok(lz4_flex::compress_hc_prepend_size(plaintext))
    }
    // decompress unchanged (HC shares the fast decoder format).
}
```

`lz4_flex` is already a workspace dependency; no new deps.

## Acceptance

- [ ] `Lz4HcCodec::compress` calls `compress_hc_prepend_size`.
- [ ] A test verifies `Lz4HcCodec` output is strictly smaller than
      `Lz4FastCodec` output on a non-RLE-friendly input (e.g. Calgary
      `paper1`).
- [ ] Cross-decode still works (HC output decodes through the fast
      decoder — wire format is identical, only compression effort
      differs).

## Why LimniFS cares

LimniFS's `max-ratio` profile would route text chunks through
LZ4 HC instead of Brotli when HC's faster decode is worth the
slightly-worse ratio. Today we can't, because HC == fast.

## Effort estimate

1 hour (one-line fix + test).
