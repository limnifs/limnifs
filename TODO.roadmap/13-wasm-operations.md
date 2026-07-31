# wasm operations

- **Priority:**  not compiled in (wasm-ops feature).
- **Depends on:** P2:02
- **Estimated effort:** see detail

## Goal

Programmable WASM operations.

## Detail

Epochs carry WASM modules that define custom operations. The module is content-addressed and included in the Merkle root. Deterministic execution via wasmi. Default

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
