# Proposal: ZSTD dictionary mode for omnizip-zstd

LimniFS's `tiny-files` benchmark (50 000 × 17-byte identical files)
currently routes through FastCDC → Brotli, achieving 69% ratio.
SquashFS achieves 43% on the same data. The gap is metadata
overhead per file. **ZSTD dictionaries** would close this gap by
training a shared dictionary on the small-file corpus and using it
as a prefix for every chunk's compression.

## The problem in numbers

| Approach | tiny-files ratio | Notes |
|---|---:|---|
| LimniFS (FastCDC + Brotli q5) | 69% | Current default |
| SquashFS (zstd L1) | 43% | Block-level compression with shared window |
| DwarFS (LZMA segmenting) | 53% | Solid compression across similar files |
| **LimniFS with ZSTD dict** (expected) | **~25%** | Trained dict on tiny-file patterns |

The ~25% estimate comes from Facebook's published results on
ZSTD dictionary mode for JSON-like workloads (small files with
shared boilerplate): typical 3-5× ratio improvement over plain
ZSTD L1.

## What omnizip needs to add

### 1. Dictionary-aware `compress`

```rust
pub fn compress_with_dict(
    plaintext: &[u8],
    level: ZstdLevel,
    dictionary: &[u8],
) -> Result<Vec<u8>, ZstdError>;
```

The `dictionary` parameter is a raw ZSTD dictionary blob (produced
by `--train` or shipped as a static asset). The encoder prepends
the dictionary's content to its match-finder hash table so the
first match can reference dictionary entries.

Implementation effort: ~200 LOC. The C reference at
`zstd/compress/zstd_compress.c:ZSTD_compress_usingDictionary` is
the spec.

### 2. Dictionary-aware `decompress`

```rust
pub fn decompress_with_dict(
    compressed: &[u8],
    expected_len: u32,
    dictionary: &[u8],
) -> Result<Vec<u8>, ZstdError>;
```

Same shape — the decoder needs the dictionary to reconstruct
matches that reference it.

Implementation effort: ~150 LOC.

### 3. Dictionary training (`ZDICT_trainFromBuffer`)

```rust
pub fn train_dictionary(
    samples: &[&[u8]],
    target_dict_size: usize,
) -> Result<Vec<u8>, ZstdError>;
```

The training algorithm finds the most common substrings across
the sample corpus and packs them into a dictionary of
`target_dict_size` bytes (typically 110 KiB). The algorithm is
covered by the "COVER" paper (Reformat et al., 2015).

Implementation effort: ~500 LOC. This is the hard part. The COVER
algorithm involves suffix-array construction + greedy substring
selection. A simpler "legacy" trainer (just take the N most common
K-byte sequences) would get 80% of the benefit at 20% of the cost.

### 4. `Codec` trait extension

```rust
pub trait Codec {
    // existing methods...

    /// Compress with an explicit dictionary. Default impl delegates
    /// to `compress` (dictionary ignored).
    fn compress_with_dict(
        &self,
        plaintext: &[u8],
        dictionary: &[u8],
    ) -> Result<Vec<u8>, OmnizipError> {
        let _ = dictionary;
        self.compress(plaintext)
    }
}
```

## Wire format implications for LimniFS

LimniFS would need to:
1. Train a per-class dictionary at image-create time (sample N
   files of each class).
2. Store the dictionary as a "shared resource" referenced by the
   slab header.
3. Encode each drop with the dictionary applied.
4. Decode with the same dictionary.

The dictionary itself becomes a content-addressed blob in the
slab. ~50-200 KiB per class. The Merkle root commits to the
dictionary's hash so tampering is detected.

**Wire-format change**: the drop record's representation byte
would need a "dictionary index" field. Current: `(codec, aead, ec)`.
New: `(codec, aead, ec, dict_id)` where `dict_id` is a u8 index
into a per-image dictionary table. 1 byte of overhead per drop.

## Acceptance criteria

1. `compress_with_dict(json_samples, Default, trained_dict)`
   produces output ≤ 50% of `compress(json_samples, Default)`
   without the dict, on a JSON-heavy corpus.
2. Round-trip preserved: decode(compress_with_dict(x, d), d) == x.
3. Dictionary size ≤ 110 KiB (ZSTD's standard).
4. No regression on existing ZSTD tests.

## Estimated omnizip effort

| Piece | LOC | Effort |
|---|---:|---|
| `compress_with_dict` | 200 | 2 days |
| `decompress_with_dict` | 150 | 1 day |
| Simple trainer (top-K substrings) | 200 | 2 days |
| COVER trainer (optimal) | 500 | 1 week |
| `Codec` trait extension | 20 | 1 hour |

**Minimum viable**: simple trainer + compress/decompress with dict
= 5 days. Gets 80% of the ratio win.

## Why this is worth it

LimniFS's tiny-files workload (and real-world equivalents like
`node_modules/`, Python `site-packages/`, Ruby `gems/`) is a
huge fraction of container-image use cases. Dictionaries are
the standard solution; SquashFS, OSTree, and Docker layer
compressors all use some form of shared-dictionary compression.

LimniFS without dictionaries is at a structural disadvantage on
this workload. With dictionaries, we'd match or beat SquashFS.

## References

- Facebook ZSTD dictionary docs: https://github.com/facebook/zstd#the-case-for-small-data-compression
- COVER algorithm paper: Reformat et al., "A greedy algorithm for the dictionary selection problem", DCC 2015.
- LimniFS tiny-files benchmark: `benchmarks/results/bench_*.md`
