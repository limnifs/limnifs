# 03: EncryptionDescriptor manifest section

## Status: IMPLEMENTED

## Scope

Add a new manifest section `encryption_descriptor` that records
which AEAD and key-wrap algorithm was used. This is the source of
truth for the reader: knowing which AEAD + key wrap → can decrypt
the slabs.

## Why

Today the AEAD algorithm is implicit in the slab header's
`crypto_hint` byte. The reader infers the AEAD from that single
byte. But with multiple AEADs (ChaCha20-Poly1305, AES-256-GCM,
AES-256-OCB), the reader needs to know which one to use — and
key wrap parameters (recipient pubkey, key wrap algo) need to be
recorded separately.

This section also enables THE stored-encryption-policy: anyone
holding the private key can decrypt, without out-of-band info.

## Design

### Manifest section

```rust
pub const ENCRYPTION_DESCRIPTOR_SECTION_VERSION: u8 = 1;

#[derive(Debug, Clone)]
pub struct EncryptionDescriptor {
    pub version: u8,
    pub aead_id: u8,         // 0x01 = ChaCha20-Poly1305, 0x02 = AES-256-GCM, 0x03 = AES-256-OCB
    pub key_wrap_id: u8,     // 0x01 = X25519+HKDF
    pub key_wrap_params: KeyWrapParams,
}

#[derive(Debug, Clone)]
pub enum KeyWrapParams {
    /// No encryption (plaintext image).
    None,
    /// X25519 ephemeral + HKDF-SHA256.
    X25519Hkdf {
        recipient_pubkey: [u8; 32],
        ephemeral_pubkey: [u8; 32],
        nonce: [u8; 24],
    },
}
```

### Wire format

```
+--------------------+  1 byte: section_version = 1
| version            |
+--------------------+  1 byte: aead_id
| aead_id            |
+--------------------+  1 byte: key_wrap_id
| key_wrap_id        |
+--------------------+  1 byte: key_wrap_params_len
| params_len         |
+--------------------+  params_len bytes: key_wrap_params
| params             |
+--------------------+
```

For `X25519Hkdf`:
- 32 bytes: recipient_pubkey
- 32 bytes: ephemeral_pubkey
- 24 bytes: nonce
- Total: 88 bytes

### API

```rust
// In limnifs-core::encryption_descriptor
pub fn parse_encryption_descriptor(cur: &mut ManifestCursor) -> Result<EncryptionDescriptor, CoreError>;
pub fn encode_encryption_descriptor(desc: &EncryptionDescriptor, out: &mut Vec<u8>);

// In limnifs-write::config
impl EncryptionConfig {
    pub fn to_encryption_descriptor(&self, recipient: Option<[u8; 32]>) -> EncryptionDescriptor;
}
```

## Implementation

1. New module `limnifs-core/src/encryption_descriptor.rs`
2. Add `EncryptionDescriptor` + `KeyWrapParams` types
3. Add parser + encoder
4. Wire into `ManifestRoot`
5. Add `EncryptionConfig::to_encryption_descriptor()` conversion
6. Update `write_directory_with_config()` to emit the section
7. Specs: round-trip each variant

## Related files

- `limnifs-core/src/aead.rs` (AEAD registry)
- `limnifs-core/src/key_wrap.rs` (key wrap module)
- `limnifs-format/src/manifest.rs` (section list)
- `limnifs-write/src/lib.rs` (assemble)
- New: `limnifs-core/src/encryption_descriptor.rs`
