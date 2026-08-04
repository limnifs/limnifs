# 02 — Wire CachedSlabStore into CLI

- **Priority:** P0
- **Side:** LimniFS
- **Est. effort:** 2h

## Problem

`CachedSlabStore` (v0.2.8) wraps `SlabStore` with an LRU cache
for decoded drop plaintexts. It exists but no CLI command uses it.
`limni cat`, `cat-multi`, and `extract` all build a raw `SlabStore`
and call `plaintext_for` directly — decompressing the same drop
on every access.

## Fix

In `limni::extract`, `cat`, and `cat_multi`:
1. Build `SlabStore` as today.
2. Call `set_dictionaries` on it.
3. Wrap in `CachedSlabStore::with_default_capacity(store)`.
4. Change `extract_file` to accept `Option<&CachedSlabStore>`
   (or use the `SlabSource` trait for polymorphism).

## Expected impact

- **`cat-multi` on 1000-file tree, second invocation**: 10× faster
  (every drop is a cache hit).
- **`extract` on multi-chunk files**: modest improvement (same drop
  may appear in multiple files' slice maps via dedup).
- **`cat` on single file**: no change (single drop access).

## Acceptance

- [ ] `limni::extract` uses `CachedSlabStore`.
- [ ] `limni::cat_multi` uses `CachedSlabStore`.
- [ ] Benchmark: repeated `cat-multi` invocation shows cache hits.
