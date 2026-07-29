# 06 — deltas-overlays

`limnifs-delta`: first-class deltas and the three merge tiers (design §7).
Read-side overlay *resolution* lives in 03; this component builds and merges.

- **Phase:** 2
- **Crate:** `limnifs-delta`
- **Design refs:** §7 (three tiers), §10.2 (delta DAG for IPFS), §16 (open question 4: rename semantics)

## Responsibilities (MECE)

**Owns:**

- Delta building: diff two trees → tree ops (add/remove/replace/rename) + drop references; delta manifest with `base_root`.
- Tier 2 — metadata-only flatten: merge N manifests into one composite manifest; drops re-referenced, never re-encoded. O(metadata).
- Tier 3 — turnover: full re-encode defrag with 04 (repack slabs, re-deepen, GC unreferenced drops); produces standalone images.
- Chain hygiene: depth policy enforcement, cycle detection at build time, GC of orphaned deltas.

**Does NOT own:** resolving overlay chains at read time (03), chunking/re-encoding mechanics (04), the delta manifest schema (01).

## Public surface

```rust
struct DeltaBuilder; // fn diff(base: &Tree, next: &Tree) -> DeltaManifest
struct Flattener;    // fn flatten(chain: &[ManifestRef]) -> Result<Manifest>   // tier 2
struct Turnover;     // fn turnover(chain: &[ManifestRef], sink: &mut dyn Sink) -> Result<Manifest> // tier 3
```

## Invariants

- Merges are identity-preserving: flattening changes no `DropId`, no file content, no metadata semantics — only reference structure.
- Tier selection is explicit or policy-driven, never silent: a flatten is always distinguishable from a turnover in the resulting manifest (`history` field).
- Meromictic chains (never flattened) are a legal permanent state; nothing forces turnover.

## Performance budget

- Flatten of depth-D chain: O(D × metadata), seconds for GB-scale trees; zero drop-store I/O.

## Tasks

- [06-delta-builder.md](06-delta-builder.md)
- [06-metadata-flatten.md](06-metadata-flatten.md)
- [06-turnover.md](06-turnover.md)
