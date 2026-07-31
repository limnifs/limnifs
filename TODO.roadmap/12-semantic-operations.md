# semantic operations

- **Priority:** P2
- **Depends on:** 02
- **Estimated effort:** see detail

## Goal

Semantic intent-based operations.

## Detail

Record WHY a change was made (UpgradePackage, ApplyPatch, SyncWith) alongside the low-level ops (Add, Replace). The semantic operation is recorded in the epoch; the low-level expansion is the proof.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
