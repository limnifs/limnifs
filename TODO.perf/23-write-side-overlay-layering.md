# 23 — Write-side overlay layering (container-image pattern)

- **Priority:** P0
- **Side:** LimniFS
- **Est. effort:** 1d
- **Status:** Done (v0.2.37)

## Problem

LimniFS has read-side overlay chain resolution (`OverlayResolver` in
`limnifs-core`), and a delta builder that computes tree-ops between
**two existing images**. But the WRITE side has no API to produce a
NEW image that is a LAYER on top of a base — referencing the base's
drops rather than re-encoding them.

This is the container-image pattern: a 1 GB base + 10 MB layer is
vastly more efficient to build, store, and distribute than a fresh
1.01 GB image. Today a LimniFS user would have to either:

1. Build the full 1.01 GB image (waste: re-encode everything)
2. Use `RwImage::open` + `commit` on the base (writes a new manifest
   but slabs grow monotonically — no cross-image drop sharing)
3. Manually diff via `compute_delta` (operates on two already-built
   images; doesn't help at write time)

## Fix

New public API:

```rust
pub fn write_layer(
    base_image: &Path,           // path to base .lim file
    root: &Path,                 // input directory (the layer's content)
    config: &WriteConfig,
) -> Result<WriteArtifact, WriteError>
```

### Algorithm

1. **Open base**: parse its manifest, mmap its slabs, build a
   `HashSet<DropId>` of every drop in the base.
2. **Walk `root`** as usual (FastCDC, hash).
3. **Per chunk**: if the DropId is in the base set, skip compress
   and emit a "referenced" drop record. Else compress and emit a
   "local" drop record into the layer's slabs.
4. **Build manifest**: metadata blob includes all inodes (regardless
   of where their drops live); drop records section includes only
   LOCAL drops; a new `base_root` field points to the base image's
   `ManifestRoot`.
5. **Output**: a `.lim` file with the layer's slabs + a manifest that
   references the base.

### Reader-side requirement (already met)

The reader resolves drop lookups across the overlay chain via
`OverlayResolver` (per spec §10.2 and the existing
`limnifs-core::overlay` module). When a drop is referenced but not
present in the current image's slabs, the resolver walks the chain.

### Slab layout

Layer slabs contain only LOCAL drops. The manifest's slab index
section includes an explicit "base references" list — drops that
must be resolved via the base. This keeps the slab byte layout
clean (no sparse slabs).

### Determinism

`write_layer` is deterministic given:
- The same `base_image` (its `ManifestRoot` is the anchor)
- The same `root` content
- The same `config`

Two different runs produce byte-identical layer images.

## Expected impact

- For 100% reused content (layer = base): near-instant (no compress)
- For container layers (mostly reused): near-instant layer creation,
  tiny layer size, fast distribution
- For 100% new content: same as `write_directory`

This is the biggest missing piece for SOTA — every modern container
image format (OCI, docker) supports layers. LimniFS should too.

## Acceptance

- [x] `write_layer` produces a `.lim` with `base_root` set
- [x] Drop records section includes only local drops (filter via
      `CODEC_REFERENCED` sentinel in `pack_slabs`)
- [x] Reader can extract the layer standalone IF all drops are local
- [x] Reader can extract the layer with base present (overlay chain)
- [x] Round-trip integrity test (`write_layer_references_base_drops`)
- [x] Benchmark: layer with 100% reused content produces 0 bytes of
      slab (verified by the test)

## Implementation notes (2026-08-06)

Shipped in v0.2.37. The implementation:

- `write_layer(base_image, root, config)` loads the base's drop set
  via `SlabStore::load_mmap` + `drop_index_keys()` (new public
  accessor).
- `WriteContext.base_drop_index: Option<HashSet<[u8;32]>>` and
  `base_root: Option<[u8;32]>` plumbed through.
- `process_file`'s chunk compress path checks the base set first;
  hits return `(drop_id, Vec::new(), Vec::new(), CODEC_REFERENCED)`.
- `pack_slabs` filters out `CODEC_REFERENCED` drops — they're never
  stored in any slab.
- `assemble` emits the `delta_linkage` section (version 1, 37 bytes)
  when `base_root` is set; the section's hash feeds the
  `SectionHashes::delta_linkage` slot.
- `write_directory_body` shared helper keeps `write_directory` and
  `write_layer` DRY.

The reader-side overlay resolution was already implemented
(`OverlayResolver` in `limnifs-core`). The writer now produces
images that exercise it.

## Implementation notes

- Plumb a `base_drop_index: Option<HashSet<[u8; 32]>>` field into
  `WriteContext`. `process_file` checks each chunk against it.
- New entry point `write_layer` is the public API. The plumbing
  inside is `WriteContext::with_base` + the existing `write_directory`
  body.
- Wire format change: `base_root` is already supported in the spec
  (delta linkage section §5.8). The new entry point emits it.

## Why this is P0

Without write-side layering, LimniFS cannot serve the OCI container
image use case efficiently. ComposeFS, the kernel-level competitor,
supports it. This is the gap that prevents LimniFS from being the
"SOTA compressed image format" for the container ecosystem.
