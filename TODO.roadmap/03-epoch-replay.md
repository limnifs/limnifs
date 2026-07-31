# 3 epoch replay

- **Priority:** P0
- **Depends on:** —
- **Estimated effort:** 1 day

## Goal

Implement epoch replay engine.

## Detail

Apply operations from epoch chain to reconstruct filesystem state at any epoch. Read-only. O(ops) per epoch. Produces a MetadataBlob + drop set equivalent to a standalone .lim image.

## Acceptance

- Spec written and implemented
- Tests cover round-trip and error cases
- CI green
