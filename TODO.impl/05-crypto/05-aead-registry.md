# 05 — AEAD registry

- **Status:** pending
- **Phase:** 2
- **Depends on:** 03-drop-store-reader
- **Design refs:** §9 (registry, nonces, AD), §4 (crypto as representation)

## Goal

`Aead` trait + registry with 0x01 XChaCha20-Poly1305 (mandatory), 0x02
AES-128-OCB, 0x03 AES-256-GCM, 0x04 Ascon-128a; HKDF deterministic nonces;
AD = manifest_root‖slab_id‖drop_index wired into the read/write paths.

## Notes

- Audited crates only, no GPL-3; constant-time guarantees documented per algorithm.
- OCB selected as the AES-NI fast path (single pass); patents abandoned, RFC 7253.
- Nonce derivation tested for position-binding: same plaintext at two positions yields different ciphertext.

## Acceptance

- Wycheproof-style vectors per algorithm pass; AD-transplant vectors fail closed.
- Identity invariant vector: encrypt/decrypt round-trip yields identical `DropId`.
