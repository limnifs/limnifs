# epoch streaming

- **Priority:** P3
- **Depends on:** 02
- **Estimated effort:** see detail

## Goal

Incremental epoch distribution.

## Detail

Stream epochs as they are produced. Each epoch is small (ops + new drops). Enables real-time filesystem sync. Content-addressed, verifiable.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
