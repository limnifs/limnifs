# 05 — Key wrap (HPKE)

- **Status:** pending
- **Phase:** 2
- **Depends on:** 05-aead-registry
- **Design refs:** §9 (keys, recipients)

## Goal

Per-recipient X25519 HPKE envelopes wrapping the image key in the manifest;
multi-recipient add/remove without re-encrypting drops.

## Notes

- Key rotation = new envelope set + optional re-wrap; drop ciphertext untouched (representation boundary).
- Plain-integrity mode (unsigned, no recipients) remains first-class for public images.

## Acceptance

- Multi-recipient vectors: each recipient opens; removed recipient fails; shared-key dedup vector passes (same plaintext drop across recipients → same `DropId`).
