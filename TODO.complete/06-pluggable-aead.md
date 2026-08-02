# 06: Pluggable AEAD — AES-256-GCM + AES-256-OCB

## Status: IMPLEMENTED

## Scope

Replace the `AeadInfo` registry with a trait-based plugin system
and add AES-256-GCM and AES-256-OCB as alternatives to ChaCha20-Poly1305.

## Why

ChaCha20-Poly1305 is excellent but:
- AES-256-GCM is hardware-accelerated on x86 (AES-NI) and ARM (ARMv8 CE)
- AES-256-OCB is faster than GCM (no separate MAC step) and
  avoids the GHASH bottleneck
- Different threat models favor different AEADs (side-channel,
  patent-free, hardware-vs-software)
- Users want choice, not a single mandated algorithm

## Design

### Trait

```rust
pub trait Aead: Send + Sync {
    fn id(&self) -> u8;
    fn name(&self) -> &'static str;
    fn key_size(&self) -> usize;
    fn nonce_size(&self) -> usize;
    fn tag_size(&self) -> usize;
    fn encrypt(
        &self,
        key: &[u8],
        nonce: &[u8],
        aad: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CoreError>;
    fn decrypt(
        &self,
        key: &[u8],
        nonce: &[u8],
        aad: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, CoreError>;
    fn overhead(&self) -> usize;
}
```

### Implementations

```rust
pub struct ChaCha20Poly1305Aead;
pub struct Aes256GcmAead;
pub struct Aes256OcbAead;
```

All three implement the same `Aead` trait. The reader looks up the
AEAD by `id` from the `encryption_descriptor` section.

### Registry

```rust
pub struct AeadRegistry {
    by_id: HashMap<u8, Box<dyn Aead>>,
}

impl AeadRegistry {
    pub fn default() -> Self;  // Registers all three
    pub fn by_id(&self, id: u8) -> Option<&dyn Aead>;
    pub fn register(&mut self, aead: Box<dyn Aead>);
}
```

### Manifest AEAD IDs

```
0x00 plaintext           (no encryption)
0x01 ChaCha20-Poly1305  (mandatory baseline)
0x02 AES-256-GCM        (new)
0x03 AES-256-OCB        (new)
```

### Dependencies

- `chacha20poly1305` (already)
- `aes-gcm` = "0.10"
- `aes` = "0.8"
- `aead` crate's OCB impl (or `ocb` crate)

## Implementation

1. New module `limnifs-core/src/aead/mod.rs` (replaces `aead.rs`)
2. Move `AeadInfo` to `aead/info.rs`
3. Add `aead/trait.rs` (Aead trait)
4. Add `aead/chacha20.rs` (impl)
5. Add `aead/aes_gcm.rs` (impl)
6. Add `aead/aes_ocb.rs` (impl)
7. Add `aead/registry.rs` (AeadRegistry)
8. Update `crypto` module to use registry
9. Specs: KAT vectors for each AEAD
10. Update `encryption_descriptor` to use the new IDs

## Related files

- `limnifs-core/src/aead.rs` (current)
- `limnifs-core/src/crypto.rs` (AEAD usage)
- New: `limnifs-core/src/aead/` (module)
