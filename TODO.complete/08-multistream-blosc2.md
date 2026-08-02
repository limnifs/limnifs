# 08: Multi-stream BLOSC2 routes

## Status: IMPLEMENTED

## Scope

Add additional BLOSC2 codec routes beyond ShuffleLz4.

## Why

`omnizip-blosc` provides multiple codec+filter combinations:
- ShuffleLz4 (already wired)
- ShuffleZstd
- BitshuffleLz4
- ShuffleZstd (high compression)

Different combinations suit different data:
- Bitshuffle is better for floating-point arrays
- Zstd is better for general text
- LZ4 is faster but worse ratio

## Design

### New codec IDs

| ID | Codec | Wire format |
|---|---|---|
| 0x0E | Shuffle+Zstd | `[u32 len][shuffled bytes][zstd compressed]` |
| 0x0F | Bitshuffle+LZ4 | `[u32 len][bitshuffle bytes][lz4 compressed]` |
| 0x10 | BZip2 (whole file) | bzip2 block format |
| 0x11 | Deflate64 (whole file) | deflate64 block format |

### Wrappers

```rust
// limnifs-core/src/codec/shuffle_zstd.rs
pub struct ShuffleZstdCodec;

// limnifs-core/src/codec/bitshuffle_lz4.rs
pub struct BitshuffleLz4Codec;

// limnifs-core/src/codec/bzip2.rs
pub struct Bzip2Codec;

// limnifs-core/src/codec/deflate64.rs
pub struct Deflate64Codec;
```

### Registration

Add to `CodecRegistry::default()`:
```rust
registry.register(CODEC_SHUFFLE_ZSTD, ShuffleZstdCodec);
registry.register(CODEC_BITSHUFFLE_LZ4, BitshuffleLz4Codec);
registry.register(CODEC_BZIP2, Bzip2Codec);
registry.register(CODEC_DEFLATE64, Deflate64Codec);
```

### Categorizers

- BZip2: route `.bz2` files to BZip2 codec (or whole-file via categorizer)
- Deflate64: route ZIP method 9 (Deflate64) files via magic detection

## Implementation

1. Add `omnizip-bzip2 = "0.11"` and `omnizip-deflate64 = "0.11"` to deps
2. Create `shuffle_zstd.rs`, `bitshuffle_lz4.rs`, `bzip2.rs`, `deflate64.rs`
3. Update `codec/mod.rs` constants and registry
4. Add a `.bz2` categorizer for BZip2
5. Specs: round-trip each

## Related files

- `limnifs-core/src/codec/mod.rs`
- `limnifs-core/src/codec/shuffle_lz4.rs` (pattern)
- `limnifs-write/src/file_categorizer/`
- New: `limnifs-core/src/codec/shuffle_zstd.rs`
- New: `limnifs-core/src/codec/bitshuffle_lz4.rs`
- New: `limnifs-core/src/codec/bzip2.rs`
- New: `limnifs-core/src/codec/deflate64.rs`
