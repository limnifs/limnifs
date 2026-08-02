# 10: mmap-based slab reads

## Status: IMPLEMENTED

## Scope

Replace `Vec<u8>` slab storage with `memmap2::Mmap` so the kernel
handles paging.

## Why

Today `SlabStore` loads entire slabs into RAM. For a 4 GB image
with 60 MB slabs, that's 4 GB of RAM. With mmap, the kernel
pages in only the bytes the reader actually reads.

For random-access workloads (locate, read_random), mmap is much
more efficient than pre-loading.

## Design

### SlabStore changes

```rust
pub struct SlabStore {
    /// One Mmap per slab.
    mmaps: Vec<Mmap>,
    /// drop_id → (slab_ordinal, offset, len) for O(1) lookup.
    drop_index: HashMap<[u8; 32], DropLocation>,
}

#[derive(Copy, Clone, Debug)]
struct DropLocation {
    slab: u16,
    offset: u32,
    len: u32,
}
```

### Adjustments

- `read_drop` returns a `&[u8]` (zero-copy view) instead of `Vec<u8>`
- `stream_drop` writes to a `Write` impl
- `plaintext_for` becomes `plaintext_view` returning `&[u8]`

## Implementation

1. Add `memmap2` dependency
2. Refactor `SlabStore` to use `Mmap`
3. Update readers to use `&[u8]` views
4. Update writers to populate mmap-ready slabs
5. Specs: correctness, memory usage vs eager

## Related files

- `limnifs-core/src/slab_store.rs`
- `limnifs-core/src/slab_reader.rs`
- `limnifs-write/src/lib.rs` (pack_slabs)
