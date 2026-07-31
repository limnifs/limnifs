# zk verification

- **Priority:** P3
- **Depends on:** 02
- **Estimated effort:** see detail

## Goal

Zero-knowledge epoch verification.

## Detail

zk-SNARK proving epoch chain validity without revealing operations. Verifier sees only Merkle roots. Pure Rust (arkworks). Opt-in.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
