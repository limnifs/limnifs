# p2p distribution

- **Priority:** P3
- **Depends on:** 02
- **Estimated effort:** see detail

## Goal

libp2p epoch gossip.

## Detail

Gossipsub protocol for P2P epoch distribution. Each peer subscribes by base image Merkle root. Opt-in via p2p feature. NOT air-gapped.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
