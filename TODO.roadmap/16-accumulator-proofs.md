# accumulator proofs

- **Priority:** P2
- **Depends on:** 02,15
- **Estimated effort:** see detail

## Goal

Cryptographic accumulator for O(1) inclusion.

## Detail

Replace O(N) chain verification with Merkle Mountain Range accumulator. O(1) proof that a file existed at a specific epoch. O(log N) chain verification.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
