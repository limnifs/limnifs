# selective disclosure

- **Priority:** P3
- **Depends on:** 02,24
- **Estimated effort:** see detail

## Goal

Selective epoch disclosure.

## Detail

Share only operations on public paths; private operations replaced with commitments. Private operations can be revealed later. Like Zcash shielded transactions for filesystems.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
