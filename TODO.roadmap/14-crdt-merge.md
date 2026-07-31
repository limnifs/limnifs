# crdt merge

- **Priority:**  any topological sort of epochs produces the same final state. Commutative, associative, idempotent.
- **Depends on:** P2:02
- **Estimated effort:** see detail

## Goal

CRDT-based multi-writer epoch merge.

## Detail

Multiple writers create epochs from the same parent. Merge is deterministic

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
