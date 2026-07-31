# 2 epoch format

- **Priority:** P0
- **Depends on:** —
- **Estimated effort:** 1 day

## Goal

Define the epoch binary format.

## Detail

Epoch header (version, parent_root, ops_hash, drops_hash, own_root, optional sig, optional timestamp) + operations list + new drops. Content-addressed: epoch ID = BLAKE3(epoch_bytes).

## Acceptance

- Spec written and implemented
- Tests cover round-trip and error cases
- CI green
