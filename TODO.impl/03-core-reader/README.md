# 03 — core-reader

`limnifs-core`: the minimal, auditable read path. Everything a mounter needs
and nothing else. no-std-adjacent; no networking; no unsafe.

- **Phase:** 0
- **Repo:** `limnifs/limnifs` (Rust workspace)
- **Crate:** `limnifs-core`
- **Design refs:** §4 (identity), §5 (three layers), §6 (read path), §11 (minimal core)

## Responsibilities (MECE)

**Owns:**

- Manifest parsing and validation (feature flags, crypto params, signatures *verification hooks* — algorithm plugs from 05).
- Drop-store reads: slab index walks, two-level addressing (slice → drop → slab extent), representation decoding via registered codecs.
- Overlay resolution: manifest-chain walking, tree-op application (add/remove/replace/rename), cycle/depth-limit rejection.
- The tier-agnostic read path: callers never know or care which tier a drop lives in.

**Does NOT own:** writing anything (04/06), network I/O (08), AEAD/EC implementations (05/07 — consumed as traits), FUSE (11).

## Public surface (traits, not structs)

```rust
trait Image { fn root(&self) -> &Manifest; fn resolve(&self, path: &Path) -> Result<Inode>; }
trait DropSource { fn read_drop(&self, id: DropId, rep: &Representation) -> Result<Bytes>; }
trait Codec { fn id(&self) -> CodecId; fn decode(&self, input: &[u8], out: &mut Vec<u8>) -> Result<()>; }
trait OverlayResolver { fn resolve_chain(&self, chain: &[ManifestRef]) -> Result<Tree>; }
```

OCP: new codecs/representations register behind `Codec`; the read loop never changes.

## Invariants

- Bounded memory: no allocation proportional to image size; slab index is paged.
- Zero-copy: decompressed output goes directly into caller-provided buffers where sizes are recorded in metadata.
- Untrusted input: every offset/length validated against slab index before any read; panics are bugs (conformance corpus hunts them).

## Performance budget

- Cold metadata open: O(manifest + slab-index pages touched), never full scan.
- Read amplification ≤ 1.25× requested extent per range read (excluding recorded solid blocks).

## Tasks

- [03-manifest-parser.md](03-manifest-parser.md)
- [03-drop-store-reader.md](03-drop-store-reader.md)
- [03-overlay-resolver.md](03-overlay-resolver.md)
- [03-python-reference-reader.md](03-python-reference-reader.md)
