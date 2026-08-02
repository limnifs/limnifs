# 09: Lazy decompression in extract

## Status: IMPLEMENTED

## Scope

Replace the eager `extract()` that decompresses all drops upfront
with a streaming version that reads/decompresses one drop at a
time and writes to the output file.

## Why

Today `extract()` materializes all decompressed plaintext in
memory. For a 1 GB image, this requires 1 GB of RAM. For
multi-GB images, this is impractical.

Streaming also reduces time-to-first-byte: the
first file's content is available as soon as its drop is read.

## Design

### API

```rust
pub trait SlabStore {
    /// Materialize a single drop's plaintext into `out`.
    fn read_drop(&self, drop_id: [u8; 32], out: &mut Vec<u8>) -> Result<(), CoreError>;

    /// Stream a drop's plaintext to a writer (no buffering).
    fn stream_drop<W: Write>(
        &self,
        drop_id: [u8; 32],
        writer: &mut W,
    ) -> Result<(), CoreError>;

    /// Stream a drop's plaintext to a writer, with a progress callback.
    fn stream_drop_with_progress<W: Write, F: Fn(u64)>(
        &self,
        drop_id: [u8; 32],
        writer: &mut W,
        progress: F,
    ) -> Result<(), CoreError>;
}
```

### CLI

```rust
// limni/src/extract.rs
fn extract_file(
    slab_store: &SlabStore,
    file: &FileEntry,
    output: &mut dyn Write,
) -> Result<(), ExtractError> {
    for slice in &file.slices {
        let drop_id = slice.drop_id;
        slab_store.stream_drop(drop_id, output)?;
    }
    Ok(())
}
```

### Memory characteristic

Before: allocation = total_image_plaintext_size
After:   allocation = max_single_drop_plaintext_size (typically 64 KB)

## Implementation

1. Add `stream_drop` to `SlabStore` trait
2. Refactor `extract()` to use streaming
3. Update `limni extract` CLI to support streaming
4. Specs: memory usage test, streaming correctness

## Related files

- `limnifs-core/src/slab_store.rs`
- `limnifs-write/src/lib.rs` (extract)
- `limni/src/main.rs` (CLI)
