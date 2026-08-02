# 12: Dict-aware FastCDC (ZSTD dict trained on first pass)

## Status: IMPLEMENTED

## Scope

The fast CDC chunker uses a static mask/seeds. Make it
dict-aware: after the first pass, train a ZSTD dict from the
collected samples, then re-classify chunks using the dict for
better entropy estimates.

## Why

The chunker currently uses raw entropy of the first 4 KB to
classify each chunk. With a trained ZSTD dict, we can compute
the "compressed size" with vs without dict — chunks that
compress significantly better with a dict are likely part of
the same vocabulary (good candidates for dict-based compression).

This feeds into the ZSTD dictionary pipeline (#11).

## Design

### Two-pass chunker

Pass 1: chunk all files with FastCDC, classify by entropy
Pass 2: 
  - For each class, train a ZSTD dict on a sample of chunks
  - Re-score each chunk's compressibility with the dict
  - Mark chunks whose dict-aided ratio is significantly better
  - Return both: chunk list + dict scores

### API

```rust
pub struct ChunkClass {
    pub class_id: u8,
    pub name: String,
    pub dict: Option<Vec<u8>>,
    pub chunks: Vec<usize>,  // indices into the file's chunk list
}

pub fn chunk_with_dicts(
    root: &Path,
    config: &WriteConfig,
) -> Result<Vec<ChunkClass>, WriteError>;
```

## Implementation

1. Refactor chunker to expose two-pass state
2. Add `omnizip_zstd::train_dictionary()` call after pass 1
3. Add `compress_with_dict()` call after pass 2 for scoring
4. Specs: dict improves ratio on repetitive code

## Related files

- `limnifs-write/src/classifier.rs`
- `limnifs-write/src/lib.rs` (chunking)
- New: `limnifs-write/src/dict_classifier.rs`
