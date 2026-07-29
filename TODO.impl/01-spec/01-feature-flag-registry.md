# 01 — Feature-flag registry and versioning policy

- **Status:** pending
- **Phase:** 0
- **Depends on:** 01-format-spec-v01
- **Design refs:** §2 (extensibility), §5 (per-layer versions)

## Goal

The feature-flag registry (data, not code): flag IDs for EC, DMS, each locator
scheme, each post-v1 codec, overlay depth semantics; plus per-layer version
numbers and the unknown-flag negotiation rules.

## Notes

- OCP backbone: post-v1 capabilities must be addable by registry row + plugin, no core change.
- Define required vs. optional flags: unknown required flag → clean `UnsupportedFeature`; unknown optional → ignore.

## Acceptance

- Conformance vectors (02) include unknown-flag cases in both classes; reference reader behaves per policy.
