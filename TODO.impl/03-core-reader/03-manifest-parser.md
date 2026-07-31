# 03 — Manifest parser

- **Status:** done — limnifs-core/src/{header,feature_flags,metadata_reference,slab_index,history,ec_params,dms_policy,delta_linkage}.rs
- **Phase:** 0
- **Depends on:** 01-flatbuffers-schema
- **Design refs:** §5 (manifest), §11 (untrusted input)

## Goal

Parse and validate the manifest: versions, feature flags, crypto params, slab
table, delta linkage (`base_root`), DMS policy, Merkle root. Expose
`Image::root()`.

## Notes

- Validation total: every offset/length bounds-checked before any read; no panics (fuzz target in 02).
- Verification hooks for signatures consume 05 traits, not concrete algorithms (OCP).
- Semantic newtypes only in the public API.

## Acceptance

- Conformance vectors: all manifest variants pass; unknown required flag → `UnsupportedFeature`; corrupt variants → structured errors, no panic.
- `clippy::pedantic` clean; no `unsafe`.
