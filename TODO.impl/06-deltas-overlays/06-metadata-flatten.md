# 06 — Metadata-only flatten (tier 2)

- **Status:** done — limnifs-write/src/flatten.rs (zero drop I/O)
- **Phase:** 2
- **Depends on:** 06-delta-builder
- **Design refs:** §7 (tier 2), §12 (CI use case: fold patches into main image)

## Goal

`Flattener::flatten(chain)`: merge N manifests into one composite manifest —
drops re-referenced, never re-encoded; result recorded as a flatten in
`history`.

## Notes

- O(metadata) only: zero drop-store I/O is the defining property (test asserts it).
- Reference rewriting must survive cross-image slab references (locator URIs carried over, 08).

## Acceptance

- Flatten of depth-3 chain on GB-scale synthetic tree completes in seconds; result resolves byte-identical to the chain (conformance vector).
- `history` distinguishes flatten from turnover (spec-defined field).
