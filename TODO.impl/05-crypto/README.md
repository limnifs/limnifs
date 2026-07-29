# 05 — crypto

`limnifs-crypto`: all cryptographic primitives behind registries. The core
reader consumes traits from here; this component never knows image semantics.

- **Phase:** 2
- **Crate:** `limnifs-crypto`
- **Design refs:** §9 (AEAD registry, nonces, AD, keys, DMS), §11 (threat model)

## Responsibilities (MECE)

**Owns:**

- AEAD registry: 0x01 XChaCha20-Poly1305 (mandatory baseline), 0x02 AES-128-OCB, 0x03 AES-256-GCM, 0x04 Ascon-128a. OCP: adding an algorithm = a registry row + a `Aead` impl.
- Deterministic nonce derivation: `nonce = HKDF(image_key, slab_id ‖ drop_index)`; associated data `manifest_root ‖ slab_id ‖ drop_index`.
- Key wrapping: per-recipient X25519 HPKE envelopes in the manifest.
- Signatures: sigstore-compatible manifest signing/verification.
- Dead man's switch primitives: time-lock puzzle seal/solve (iterated squaring), Shamir k-of-n escrow split/collect. Policy storage is manifest schema (01); this crate provides the math.

**Does NOT own:** when encryption is applied (representation pipeline, 04), key storage UX (10), escrow *policy* (01 schema + 10 CLI flows).

## Public surface

```rust
trait Aead { fn id(&self) -> AeadId; fn seal(&self, key: &Key, nonce: &Nonce, ad: &[u8], pt: &[u8]) -> Vec<u8>; fn open(...) -> Result<Vec<u8>>; }
trait KeyWrap { fn wrap(&self, image_key: &Key, recipient: &PublicKey) -> Envelope; fn unwrap(...) -> Result<Key>; }
trait Signer { fn sign_manifest(&self, root: ManifestRoot) -> Signature; }
trait Dms { fn seal(&self, key: &Key, policy: &DmsPolicy) -> DmsRecord; fn solve(&self, rec: &DmsRecord) -> Result<Key>; }
```

## Invariants

- Identity rule preserved: identity is BLAKE3 of *plaintext*; crypto is a representation (design §4). Dedup works across recipients sharing an image key.
- No nonce reuse by construction: nonces derive from position, never from counters or randomness.
- Transplant resistance: AD binds every sealed drop to (image, slab, position).
- No GPL-3 dependencies; constant-time implementations only (audited crates: `chacha20poly1305`, `aes-gcm`, `ascon` or vetted equivalents).

## Tasks

- [05-aead-registry.md](05-aead-registry.md)
- [05-key-wrap-hpke.md](05-key-wrap-hpke.md)
- [05-signing-sigstore.md](05-signing-sigstore.md)
- [05-dms.md](05-dms.md)
