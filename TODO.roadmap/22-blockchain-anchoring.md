# blockchain anchoring

- **Priority:** P3
- **Depends on:** 02,08
- **Estimated effort:** see detail

## Goal

Blockchain-anchored timestamps.

## Detail

Anchor epoch Merkle roots to Bitcoin (OpenTimestamps, free) or Ethereum (contract call). Opt-in. Proves epoch existed at block height. NOT air-gapped.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
