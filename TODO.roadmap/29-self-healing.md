# self healing

- **Priority:**  epoch chain identifies which drops were in the slab, fetches from mirrors via locators, reconstructs, verifies Merkle chain still valid.
- **Depends on:** P3:02
- **Estimated effort:** see detail

## Goal

Self-healing via epoch chain.

## Detail

Corrupted slab recovery

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
