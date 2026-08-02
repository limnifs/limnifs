# 13: AES-256-OCB support

## Status: IMPLEMENTED (limnifs-ocb3 crate)

## Scope

Add `Aes256OcbAead` as a third AEAD option (alongside
ChaCha20-Poly1305 and AES-256-GCM). Wire it into the AEAD
registry.

## Why

OCB (Offset Codebook Mode) is faster than GCM:
- Single pass encryption (vs GCM's encrypt-then-MAC)
- No GHASH bottleneck
- Constant-time (side-channel resistant)
- Patent-free (RFC 7253, NIST approved)

For environments without AES-NI but with good SIMD, OCB often
beats both GCM and ChaCha20-Poly1305.

## Design

### Wire format

Same as other AEADs: nonce + ciphertext + tag.
- Key: 32 bytes
- Nonce: 12-16 bytes (we use 12 for compatibility)
- Tag: 16 bytes

### Registry entry

```rust
const AEAD_AES_256_OCB: u8 = 0x03;

struct Aes256OcbAead;

impl Aead for Aes256OcbAead {
    fn id(&self) -> u8 { AEAD_AES_256_OCB }
    fn name(&self) -> &'static str { "AES-256-OCB" }
    fn key_size(&self) -> usize { 32 }
    fn nonce_size(&self) -> usize { 12 }
    fn tag_size(&self) -> usize { 16 }
    fn encrypt(&self, key, nonce, aad, plaintext) -> Result<Vec<u8>, CoreError> { ... }
    fn decrypt(&self, key, nonce, aad, ciphertext) -> Result<Vec<u8>, CoreError> { ... }
}
```

### Dependencies

- `aead` crate (0.5.x) with `ocb` feature, or
- `ocb` crate

## Implementation

1. Add `aead` or `ocb` dependency
2. Create `limnifs-core/src/aead/aes_ocb.rs`
3. Register in `AeadRegistry::default()`
4. Specs: KAT vectors

## Related files

- `limnifs-core/src/aead/` (module)
- `limnifs-core/Cargo.toml`
