# 12 — Parity test suite

- **Status:** pending
- **Phase:** 1
- **Depends on:** 12-press-consume-limni, 09-import-dwarfs
- **Design refs:** §15 (Phase 1 exit gate)

## Goal

Same app packaged via dwarfs-t and via LimniFS: diff mounted trees, compare
cold start, image size, peak RSS; plus migrated-image parity (import-dwarfs
output vs. original mount).

## Notes

- The Phase 1 exit gate: "tebako packages and runs from LimniFS images with parity vs dwarfs-t."
- Numbers land in this file (recorded, not claimed) and feed the design doc's §1.1 claims.

## Acceptance

- Tree diff: empty. Cold start: ≤ dwarfs-t. Size: ≤ dwarfs-t at comparable settings. All three recorded for the fixture set.
