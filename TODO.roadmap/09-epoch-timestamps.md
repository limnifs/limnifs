# 09 — Epoch timestamps (local + opt-in external)

- **Priority:** P1
- **Depends on:** 02-epoch-format
- **Estimated effort:** 3 hours

## Goal

Each epoch carries a timestamp. Three modes:

1. **Local (default, air-gapped)**: system clock at commit time +
   signer's Ed25519 attestation. Trust = "the signer says when".
2. **RFC 3161 TSA (opt-in)**: the timestamp is countersigned by a
   Time Stamping Authority. Trust = "the TSA says when". Requires
   network to reach the TSA.
3. **Blockchain anchoring (opt-in, task 22)**: epoch Merkle root is
   anchored to Bitcoin/Ethereum. Trust = "the blockchain proves when".

## Acceptance

- Local timestamps work with no network
- RFC 3161 is opt-in via `--tsa-url <url>`
- Timestamp is part of the Merkle chain (tampering breaks verification)
