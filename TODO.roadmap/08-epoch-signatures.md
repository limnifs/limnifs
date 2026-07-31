# 08 — Per-epoch Ed25519 signatures

- **Priority:** P1
- **Depends on:** 02-epoch-format, signing feature
- **Estimated effort:** 3 hours

## Goal

Each epoch carries an optional Ed25519 signature over (parent_root ||
ops_hash || drops_hash || timestamp). The signer's public key is
included in the epoch. Verification is offline (no network).

## Design

```
Epoch {
  ...
  signature: Option<([u8;32] pubkey, [u8;64] sig)>,
}
```

`limni commit --sign <key-file>` signs the epoch.
`limni verify-epoch <epoch>` checks signature + Merkle chain.

## Air-gapped

Default: unsigned epochs (trust the Merkle chain only).
With signing feature: Ed25519 signatures, fully offline.
Opt-in: sigstore keyless via cosign (shell-out, network needed).

## Acceptance

- Signed epochs verify offline
- Unsigned epochs accepted (backward compatible)
- Tampered epoch rejected (signature mismatch)
