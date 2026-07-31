# seal operation

- **Priority:** P1
- **Depends on:** 02,10
- **Estimated effort:** see detail

## Goal

Seal operation for WORM compliance.

## Detail

Seal() marks an epoch as final. No further epochs can chain from a sealed epoch without breaking verification. Satisfies SEC Rule 17a-4 WORM requirement.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
