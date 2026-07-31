# branching

- **Priority:** P2
- **Depends on:** 02
- **Estimated effort:** see detail

## Goal

Fork-free epoch branching.

## Detail

limni branch --from-epoch K --name feature. New epochs chain from epoch K. Shared drops automatically deduplicate. Merges via CRDT rules.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
