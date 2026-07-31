# 4 epoch commit

- **Priority:** P0
- **Depends on:** —
- **Estimated effort:** 1 day

## Goal

Implement limni commit.

## Detail

Take an overlay directory + base .lim image, diff them, produce an epoch file with the operations + new drops. Writes to <image>.epoch-N.

## Acceptance

- Spec written and implemented
- Tests cover round-trip and error cases
- CI green
