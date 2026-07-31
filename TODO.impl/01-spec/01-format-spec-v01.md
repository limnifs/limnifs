# 01 — Format spec v0.1

- **Status:** done — spec lives in limnifs/spec repo; bit-level docs in limnifs/spec/bit-level/
- **Phase:** 0
- **Depends on:** none
- **Design refs:** §4, §5, §9, §10.1

## Goal

`spec/SPEC.md`: complete prose specification of the three layers (drop store,
metadata, manifest), the identity rule, two-level addressing, overlay chain
semantics, and versioning rules.

## Notes

- Written before any implementation code (spec-first; code follows spec).
- Every field semantic defined here; the `.fbs` schema (01-flatbuffers-schema) must match.
- Include worked examples: single image, delta chain of depth 2, encrypted image, EC image.

## Acceptance

- Spec is self-sufficient: a reader can be implemented from it alone (validated by 03-python-reference-reader being written from spec, not from Rust code).
- Review checklist: no unspecified byte, no ambiguous ordering rule, unknown-flag behavior stated.
