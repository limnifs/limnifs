//! AEAD operations: encrypt/decrypt trait + implementations.
//!
//! | Id | Name | Crate | Notes |
//! |---|---|---|---|
//! | 0x01 | XChaCha20-Poly1305 | `chacha20poly1305` | 24-byte nonce, mandatory baseline |
//! | 0x02 | AES-256-GCM | `aes-gcm` | 12-byte nonce, hardware-accelerated |
//! | 0x03 | AES-256-OCB | `ocb3` + `aes` | 12-byte nonce, RFC 7253 |

use crate::aead::{AEAD_AES_256_GCM, AEAD_AES_256_OCB, AEAD_XCHACHA20_POLY1305};
use crate::error::CoreError;

/// Authenticated encryption with associated data.
pub trait Aead: Send + Sync {
    fn id(&self) -> u8;
    fn name(&self) -> &'static str;
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
}

// ── XChaCha20-Poly1305 ────────────────────────────────────────

pub struct XChaCha20Poly1305Aead;

impl Aead for XChaCha20Poly1305Aead {
    fn id(&self) -> u8 {
        AEAD_XCHACHA20_POLY1305
    }
    fn name(&self) -> &'static str {
        "XChaCha20-Poly1305"
    }

    fn encrypt(
        &self,
        key: &[u8],
        nonce: &[u8],
        aad: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CoreError> {
        use chacha20poly1305::aead::{Aead, KeyInit, Payload};
        if key.len() != 32 {
            return Err(key_err("xchacha20", 32, key.len()));
        }
        if nonce.len() != 24 {
            return Err(nonce_err("xchacha20", 24, nonce.len()));
        }
        let cipher = chacha20poly1305::XChaCha20Poly1305::new_from_slice(key).map_err(|e| {
            CoreError::Corrupt {
                reason: format!("xchacha20: {e}"),
            }
        })?;
        let payload = Payload {
            msg: plaintext,
            aad,
        };
        cipher
            .encrypt(
                chacha20poly1305::aead::generic_array::GenericArray::from_slice(nonce),
                payload,
            )
            .map_err(|_| CoreError::Corrupt {
                reason: "xchacha20: encrypt failed".into(),
            })
    }

    fn decrypt(
        &self,
        key: &[u8],
        nonce: &[u8],
        aad: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, CoreError> {
        use chacha20poly1305::aead::{Aead, KeyInit, Payload};
        let cipher = chacha20poly1305::XChaCha20Poly1305::new_from_slice(key).map_err(|e| {
            CoreError::Corrupt {
                reason: format!("xchacha20: {e}"),
            }
        })?;
        let payload = Payload {
            msg: ciphertext,
            aad,
        };
        cipher
            .decrypt(
                chacha20poly1305::aead::generic_array::GenericArray::from_slice(nonce),
                payload,
            )
            .map_err(|_| CoreError::Corrupt {
                reason: "xchacha20: decrypt failed".into(),
            })
    }
}

// ── AES-256-GCM ───────────────────────────────────────────────

pub struct Aes256GcmAead;

impl Aead for Aes256GcmAead {
    fn id(&self) -> u8 {
        AEAD_AES_256_GCM
    }
    fn name(&self) -> &'static str {
        "AES-256-GCM"
    }

    fn encrypt(
        &self,
        key: &[u8],
        nonce: &[u8],
        aad: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CoreError> {
        use aes_gcm::aead::{Aead, KeyInit, Payload};
        use aes_gcm::Nonce;
        if key.len() != 32 {
            return Err(key_err("aes-gcm", 32, key.len()));
        }
        if nonce.len() != 12 {
            return Err(nonce_err("aes-gcm", 12, nonce.len()));
        }
        let cipher = aes_gcm::Aes256Gcm::new_from_slice(key).map_err(|e| CoreError::Corrupt {
            reason: format!("aes-gcm: {e}"),
        })?;
        let payload = Payload {
            msg: plaintext,
            aad,
        };
        cipher
            .encrypt(Nonce::from_slice(nonce), payload)
            .map_err(|_| CoreError::Corrupt {
                reason: "aes-gcm: encrypt failed".into(),
            })
    }

    fn decrypt(
        &self,
        key: &[u8],
        nonce: &[u8],
        aad: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, CoreError> {
        use aes_gcm::aead::{Aead, KeyInit, Payload};
        use aes_gcm::Nonce;
        let cipher = aes_gcm::Aes256Gcm::new_from_slice(key).map_err(|e| CoreError::Corrupt {
            reason: format!("aes-gcm: {e}"),
        })?;
        let payload = Payload {
            msg: ciphertext,
            aad,
        };
        cipher
            .decrypt(Nonce::from_slice(nonce), payload)
            .map_err(|_| CoreError::Corrupt {
                reason: "aes-gcm: decrypt failed".into(),
            })
    }
}

// ── AES-256-OCB (RFC 7253, via limnifs-ocb3) ──────────────────

pub struct Aes256OcbAead;

impl Aead for Aes256OcbAead {
    fn id(&self) -> u8 {
        AEAD_AES_256_OCB
    }
    fn name(&self) -> &'static str {
        "AES-256-OCB"
    }

    fn encrypt(
        &self,
        key: &[u8],
        nonce: &[u8],
        aad: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CoreError> {
        if key.len() != 32 {
            return Err(key_err("aes-ocb", 32, key.len()));
        }
        if nonce.len() != 12 {
            return Err(nonce_err("aes-ocb", 12, nonce.len()));
        }
        let key_arr: &[u8; 32] = key.try_into().map_err(|_| CoreError::Corrupt {
            reason: "aes-ocb: key length assertion failed".into(),
        })?;
        let nonce_arr: &[u8; 12] = nonce.try_into().map_err(|_| CoreError::Corrupt {
            reason: "aes-ocb: nonce length assertion failed".into(),
        })?;
        let ocb = limnifs_ocb3::Ocb3Aes256::new(key_arr);
        let mut buf = plaintext.to_vec();
        let tag = ocb.encrypt_in_place_detached(nonce_arr, aad, &mut buf);
        buf.extend_from_slice(&tag);
        Ok(buf)
    }

    fn decrypt(
        &self,
        key: &[u8],
        nonce: &[u8],
        aad: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, CoreError> {
        if ciphertext.len() < 16 {
            return Err(CoreError::Corrupt {
                reason: "aes-ocb: ciphertext too short for tag".into(),
            });
        }
        if key.len() != 32 || nonce.len() != 12 {
            return Err(CoreError::Corrupt {
                reason: "aes-ocb: invalid key/nonce length".into(),
            });
        }
        let key_arr: &[u8; 32] = key.try_into().unwrap();
        let nonce_arr: &[u8; 12] = nonce.try_into().unwrap();
        let ocb = limnifs_ocb3::Ocb3Aes256::new(key_arr);
        let tag_start = ciphertext.len() - 16;
        let mut buf = ciphertext[..tag_start].to_vec();
        let mut tag = [0u8; 16];
        tag.copy_from_slice(&ciphertext[tag_start..]);
        ocb.decrypt_in_place_detached(nonce_arr, aad, &mut buf, &tag)
            .map_err(|()| CoreError::Corrupt {
                reason: "aes-ocb: decrypt failed (tag mismatch?)".into(),
            })?;
        Ok(buf)
    }
}

// ── Registry ──────────────────────────────────────────────────

pub struct AeadRegistry {
    by_id: Vec<Box<dyn Aead>>,
}

impl AeadRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self { by_id: Vec::new() }
    }

    #[must_use]
    pub fn default_registry() -> Self {
        let mut r = Self::new();
        r.register(Box::new(XChaCha20Poly1305Aead));
        r.register(Box::new(Aes256GcmAead));
        r.register(Box::new(Aes256OcbAead));
        r
    }

    pub fn register(&mut self, aead: Box<dyn Aead>) {
        self.by_id.push(aead);
    }

    #[must_use]
    pub fn get(&self, id: u8) -> Option<&dyn Aead> {
        self.by_id
            .iter()
            .find(|a| a.id() == id)
            .map(std::convert::AsRef::as_ref)
    }
}

impl Default for AeadRegistry {
    fn default() -> Self {
        Self::default_registry()
    }
}

impl std::fmt::Debug for AeadRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AeadRegistry")
            .field(
                "ids",
                &self.by_id.iter().map(|a| a.id()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

fn key_err(name: &str, expected: usize, got: usize) -> CoreError {
    CoreError::Corrupt {
        reason: format!("{name}: key must be {expected} bytes, got {got}"),
    }
}

fn nonce_err(name: &str, expected: usize, got: usize) -> CoreError {
    CoreError::Corrupt {
        reason: format!("{name}: nonce must be {expected} bytes, got {got}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [0x42; 32];
    const NONCE_12: [u8; 12] = [0xAB; 12];
    const NONCE_24: [u8; 24] = [0xCD; 24];

    #[test]
    fn xchacha20_round_trip() {
        let aead = XChaCha20Poly1305Aead;
        let ct = aead
            .encrypt(&KEY, &NONCE_24, b"aad", b"plaintext")
            .expect("encrypt");
        let pt = aead.decrypt(&KEY, &NONCE_24, b"aad", &ct).expect("decrypt");
        assert_eq!(pt, b"plaintext");
    }

    #[test]
    fn aes_256_gcm_round_trip() {
        let aead = Aes256GcmAead;
        let ct = aead
            .encrypt(&KEY, &NONCE_12, b"aad", b"plaintext")
            .expect("encrypt");
        let pt = aead.decrypt(&KEY, &NONCE_12, b"aad", &ct).expect("decrypt");
        assert_eq!(pt, b"plaintext");
    }

    #[test]
    fn aes_256_ocb_round_trip() {
        let aead = Aes256OcbAead;
        let ct = aead
            .encrypt(&KEY, &NONCE_12, b"aad", b"plaintext")
            .expect("encrypt");
        assert!(ct.len() >= 16);
        let pt = aead.decrypt(&KEY, &NONCE_12, b"aad", &ct).expect("decrypt");
        assert_eq!(pt, b"plaintext");
    }

    #[test]
    fn registry_default_has_all_three() {
        let registry = AeadRegistry::default_registry();
        assert!(registry.get(AEAD_XCHACHA20_POLY1305).is_some());
        assert!(registry.get(AEAD_AES_256_GCM).is_some());
        assert!(registry.get(AEAD_AES_256_OCB).is_some());
    }

    #[test]
    fn detects_tampered_ciphertext() {
        let aead = Aes256GcmAead;
        let mut ct = aead
            .encrypt(&KEY, &NONCE_12, b"", b"secret")
            .expect("encrypt");
        ct[0] ^= 0xFF;
        assert!(aead.decrypt(&KEY, &NONCE_12, b"", &ct).is_err());
    }
}
