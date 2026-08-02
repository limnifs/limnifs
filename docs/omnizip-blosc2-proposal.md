# Proposal: BLOSC2 container + shuffle for omnizip-rs

BLOSC2 is a multi-codec container format designed for
**scientific data**: NumPy arrays, multi-dimensional grids,
columnar databases. It applies a shuffle (byte or bit) before
the inner codec (LZ4, ZSTD), which dramatically improves ratio
on multi-byte numeric data where adjacent values are similar
(time series, sensor streams, FITS pixel data).

DwarFS does not have BLOSC. SquashFS does not have BLOSC.
LimniFS would be **unique** in the filesystem-image space if we
integrated it.

## The problem in numbers

Current LimniFS on scientific data:

| Data type | LimniFS | DwarFS | SquashFS | tar+zstd |
|---|---:|---:|---:|---:|
| FITS 16-bit pixels (integer) | **32.08%** | 89.97% | 90.18% | 85.69% |
| Float32 scientific (estimated) | ~80% | ~85% | ~85% | ~80% |

ricepp handles the integer case well (32%). But ricepp can't do
floating-point data — climate models, CFD outputs, ML weights
all store as float32/float64 and currently get ~80% (barely
better than store).

BLOSC2 with shuffle + LZ4 would get:
- Float32 arrays: 30-50% (shuffle exposes the mantissa/exponent
  patterns that LZ4 can then match).
- Float64 arrays: 40-60%.

## What omnizip needs to add

### New crate: `omnizip-blosc`

```
omnizip-blosc/
├── Cargo.toml
├── src/
│   ├── lib.rs            # Codec trait impl, public API
│   ├── container.rs      # BLOSC2 frame format (header + chunks)
│   ├── shuffle.rs        # Byte-shuffle + bit-shuffle filters
│   └── inner_codec.rs    # Wrap LZ4/ZSTD for the chunk body
└── tests/
    └── differential.rs   # Round-trip on numpy float arrays
```

### Public API

```rust
pub fn compress(
    input: &[u8],
    item_size: u8,      // 1, 2, 4, or 8 bytes per element
    shuffle: Shuffle,   // None, ByteShuffle, BitShuffle
    inner_codec: InnerCodec,  // Lz4, Zstd
) -> Result<Vec<u8>, OmnizipError>;

pub fn decompress(compressed: &[u8]) -> Result<Vec<u8>, OmnizipError>;

pub enum Shuffle { None, Byte, Bit }
pub enum InnerCodec { Lz4, Zstd }
```

The `item_size` parameter is the key: BLOSC2 assumes the input is
a sequence of fixed-size records (e.g. f32 = 4 bytes each). The
shuffle transposes byte-lanes: all byte-0s together, all byte-1s
together, etc. After shuffle, byte-0s are highly correlated
(mantissa high bits) and compress well; byte-3s are noisy
(exponent low bits) and get stored verbatim.

### Wire format

BLOSC2's frame format is:
```
[header (32 bytes)]
  - magic (4B)
  - version (2B)
  - item_size (1B)
  - shuffle (1B)
  - inner_codec (1B)
  - uncompressed_size (8B)
  - chunk_size (4B)
  - chunk_count (4B)
  - reserved (7B)
[chunks...]
  per-chunk: [chunk_header (8B)] [shuffled_data] [compressed_data]
```

Self-describing — decoder knows everything from the header.

## LimniFS integration

LimniFS would add:
- **New codec id 0x0A** = BLOSC2.
- **New file categorizer** `scientific.rs` that detects NumPy
  `.npy` magic, FITS float32 BITPIX (-32 or -64), and HDF5 magic.
  Routes these to BLOSC2 with the right `item_size`.
- **Codec wrapper** at `limnifs-core/src/codec/blosc.rs`.

Estimated LimniFS-side effort: ~200 LOC.

## Acceptance criteria

1. `compress(float32_array, 4, Byte, Lz4)` produces output ≤ 50%
   of `lz4_compress(float32_array)` on a synthetic smooth-float
   dataset.
2. Round-trip preserved.
3. Bit-shuffle beats byte-shuffle on float32 data (more entropy
   exposed).
4. Differential test against Python's `blosc2.compress()` on the
   same input produces byte-identical shuffled bytes (the inner
   LZ4 can differ — only the shuffle layer needs to match).

## Why this is worth it

Scientific data workloads are a growing segment (ML weights,
climate models, genomics). LimniFS's content-addressed model is
a natural fit for reproducible scientific computing — every
dataset has a stable identity across re-compression. BLOSC2
would let us serve that use case better than any competing
filesystem image format.

## Estimated effort

| Piece | LOC | Effort |
|---|---:|---|
| Shuffle (byte + bit) | 400 | 3 days |
| Container format | 300 | 2 days |
| LZ4 inner codec wrap | 100 | 1 day |
| ZSTD inner codec wrap | 100 | 1 day |
| Differential tests | 200 | 2 days |
| `Codec` trait + limnifs wrapper | 200 | 1 day |

**Total**: ~2 weeks. Smaller than FLAC LPC port; comparable to
ricepp.

## Lower-effort alternative

If the full BLOSC2 port is too much, a **shuffle-only preprocessor**
(compose with existing LZ4/ZSTD) gets ~70% of the win:

```rust
// In omnizip-filters (already exists)
pub fn shuffle(input: &[u8], item_size: usize) -> Vec<u8>;
pub fn unshuffle(input: &[u8], item_size: usize) -> Vec<u8>;
```

LimniFS would compose: `shuffle → lz4_compress` on encode,
`lz4_decompress → unshuffle` on decode. No new codec id needed —
just a new filter in `omnizip-filters`.

This is ~200 LOC total. Same wire format as a "filtered LZ4"
codec.

## References

- BLOSC2 source: https://github.com/Blosc/c-blosc2
- Bit-shuffle paper: https://github.com/kiyo-masui/bitshuffle
- LimniFS FITS benchmark: `benchmarks/results/bench_*.md` (fits-synthetic)
- DwarFS source (no BLOSC): `src/external/dwarfs-t/src/compression/`
