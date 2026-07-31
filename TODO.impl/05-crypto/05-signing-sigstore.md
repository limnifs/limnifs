# 05 — Sigstore manifest signing

- **Status:** done — keypair mode in limnifs-core/src/signing.rs; keyless via limni sigstore-sign (cosign)
- **Phase:** 2
- **Depends on:** 05-aead-registry
- **Design refs:** §9 (signatures), §11 (supply chain)

## Goal

Sign/verify of the manifest root with sigstore-compatible bundles (keyless
Fulcio + Rekor, and plain keypair mode), surfaced in `limni verify`.

## Notes

- Signature covers `ManifestRoot` + policy section; drops transitively covered by the Merkle tree.
- Offline verification possible (bundled cert chain + transparency proof).

## Acceptance

- Sign→verify round-trip vectors; tampered-manifest rejection vectors; offline verify vector without network.
