# 07: Codec size thresholds

## Status: IMPLEMENTED

## Scope

Add a `min_compress_size` field to the `Codec` trait. The writer's
compression tournament skips codecs whose `min_compress_size` is
greater than the chunk size.

## Why

PPMd (ZPAQ coder + 1M-slot model) has a high fixed cost per chunk.
Trying it on 256-byte chunks is wasteful — the model setup alone
exceeds 256 bytes. Same for ZPAQ and other context-modeling codecs.

Setting per-codec thresholds lets the writer skip expensive codecs
on small inputs.

## Design

### Trait change

```rust
pub trait Codec: Send + Sync {
    fn id(&self) -> u8;
    fn name(&self) -> &'static str;
    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError>;
    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError>;

    /// Minimum input size for this codec to be tried in the tournament.
    /// Chunks smaller than this skip the codec entirely.
    /// Default: 0 (no threshold).
    fn min_compress_size(&self) -> usize { 0 }
}
```

### Per-codec thresholds

| Codec | min_compress_size | Rationale |
|---|---|---|
| store | 0 | always |
| lz4 | 0 | always |
| zstd | 0 | always |
| brotli | 0 | always |
| deflate | 0 | always |
| snappy | 0 | always |
| flac | 1024 | FLAC framing overhead per frame |
| ricepp | 1024 | Rice+ per-block overhead |
| fsst+brotli | 256 | FSST table overhead |
| shuffle+lz4 | 512 | Shuffle per-block overhead |
| zpaq | 4096 | Multi-model initialization cost |
| ppmd | 4096 | Adaptive model table cost |
| glza | 4096 | Grammar construction cost |

## Implementation

1. Add `min_compress_size()` to `Codec` trait with default impl of 0
2. Override in each codec impl where threshold > 0
3. Update tournament logic in `limnifs-write/src/lib.rs`:
   ```rust
   if chunk.len() < codec.min_compress_size() {
       continue; // skip this codec
   }
   ```
4. Specs: tournament skips correctly for small inputs

## Related files

- `limnifs-core/src/codec/mod.rs` (Codec trait)
- `limnifs-core/src/codec/*` (per-codec impls)
- `limnifs-write/src/lib.rs` (tournament logic)
