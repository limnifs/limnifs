# parallel replay

- **Priority:** P2
- **Depends on:** 03
- **Estimated effort:** see detail

## Goal

Parallel epoch application via rayon.

## Detail

Epochs are parsed in parallel. Operations on different paths are applied in parallel. Synchronize only when operations touch the same path.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
