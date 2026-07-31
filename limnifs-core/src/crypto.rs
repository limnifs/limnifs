//! AEAD encrypt/decrypt — actual cryptographic operations.
//!
//! Uses the [`crate::aead`] registry for metadata dispatch. Currently
//! supports:
//!
//! - `0x00` plaintext: no-op (returns input unchanged)
//! - `0x01` `XChaCha20-Poly1305`: authenticated encryption with
//!   extended nonce
//!
//! ## `XChaCha20-Poly1305` parameters
//!
//! | Parameter | Value |
//! |---|---|
//! | Key | 32 bytes |
//! | Nonce | 24 bytes |
//! | Tag | 16 bytes (appended to ciphertext) |
//! | AAD | optional, authenticated but not encrypted |

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::aead::{lookup, AEAD_PLAINTEXT, AEAD_XCHACHA20_POLY1305};
use crate::error::CoreError;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

/// Seal (encrypt + authenticate) `plaintext` using the AEAD identified
/// by `aead_id`. Returns `ciphertext || tag` (the tag is appended).
///
/// For plaintext AEAD (`0x00`), the input is returned unchanged.
///
/// # Errors
///
/// - [`CoreError::UnsupportedFeature`] for unknown AEAD ids.
/// - [`CoreError::Corrupt`] if the key or nonce sizes are wrong, or
///   encryption fails.
pub fn seal(
    aead_id: u8,
    key: &[u8],
    nonce: &[u8],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, CoreError> {
    match aead_id {
        AEAD_PLAINTEXT => Ok(plaintext.to_vec()),
        AEAD_XCHACHA20_POLY1305 => {
            let info = lookup(aead_id).ok_or_else(|| CoreError::UnsupportedFeature {
                feature: format!("seal AEAD 0x{aead_id:02X}"),
            })?;
            validate_key_nonce(aead_id, key, nonce, info.key_size, info.nonce_size)?;
            let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
            cipher
                .encrypt(
                    XNonce::from_slice(nonce),
                    Payload {
                        msg: plaintext,
                        aad,
                    },
                )
                .map_err(|e| CoreError::Corrupt {
                    reason: format!("XChaCha20-Poly1305 seal failed: {e}"),
                })
        }
        other => Err(CoreError::UnsupportedFeature {
            feature: format!("seal AEAD 0x{other:02X}"),
        }),
    }
}

/// Open (decrypt + verify) `ciphertext_with_tag` using the AEAD
/// identified by `aead_id`. The tag is the last 16 bytes of the input.
///
/// For plaintext AEAD (`0x00`), the input is returned unchanged.
///
/// # Errors
///
/// - [`CoreError::UnsupportedFeature`] for unknown AEAD ids.
/// - [`CoreError::Corrupt`] if the key or nonce sizes are wrong, or
///   decryption/authentication fails (wrong key, tampered data, etc.).
pub fn open(
    aead_id: u8,
    key: &[u8],
    nonce: &[u8],
    ciphertext_with_tag: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, CoreError> {
    match aead_id {
        AEAD_PLAINTEXT => Ok(ciphertext_with_tag.to_vec()),
        AEAD_XCHACHA20_POLY1305 => {
            let info = lookup(aead_id).ok_or_else(|| CoreError::UnsupportedFeature {
                feature: format!("open AEAD 0x{aead_id:02X}"),
            })?;
            validate_key_nonce(aead_id, key, nonce, info.key_size, info.nonce_size)?;
            let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
            cipher
                .decrypt(
                    XNonce::from_slice(nonce),
                    Payload {
                        msg: ciphertext_with_tag,
                        aad,
                    },
                )
                .map_err(|e| CoreError::Corrupt {
                    reason: format!("XChaCha20-Poly1305 open failed: {e}"),
                })
        }
        other => Err(CoreError::UnsupportedFeature {
            feature: format!("open AEAD 0x{other:02X}"),
        }),
    }
}

fn validate_key_nonce(
    aead_id: u8,
    key: &[u8],
    nonce: &[u8],
    expected_key: usize,
    expected_nonce: usize,
) -> Result<(), CoreError> {
    if key.len() != expected_key {
        return Err(CoreError::Corrupt {
            reason: format!(
                "AEAD 0x{aead_id:02X}: key size {} != expected {expected_key}",
                key.len()
            ),
        });
    }
    if nonce.len() != expected_nonce {
        return Err(CoreError::Corrupt {
            reason: format!(
                "AEAD 0x{aead_id:02X}: nonce size {} != expected {expected_nonce}",
                nonce.len()
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> Vec<u8> {
        vec![0x42u8; 32]
    }

    fn test_nonce() -> Vec<u8> {
        vec![0x01u8; 24]
    }

    #[test]
    fn plaintext_seal_is_identity() {
        let data = b"hello world";
        let sealed = seal(AEAD_PLAINTEXT, &[], &[], data, &[]).expect("plaintext seal");
        assert_eq!(sealed, data);
    }

    #[test]
    fn plaintext_open_is_identity() {
        let data = b"hello world";
        let opened = open(AEAD_PLAINTEXT, &[], &[], data, &[]).expect("plaintext open");
        assert_eq!(opened, data);
    }

    #[test]
    fn xchacha20_round_trips() {
        let key = test_key();
        let nonce = test_nonce();
        let plaintext = b"secret message for encryption";
        let aad = b"associated data";

        let sealed =
            seal(AEAD_XCHACHA20_POLY1305, &key, &nonce, plaintext, aad).expect("seal succeeds");
        assert_ne!(&sealed[..], plaintext); // must be different
        assert_eq!(sealed.len(), plaintext.len() + 16); // ciphertext + 16-byte tag

        let opened =
            open(AEAD_XCHACHA20_POLY1305, &key, &nonce, &sealed, aad).expect("open succeeds");
        assert_eq!(opened, plaintext);
    }

    #[test]
    fn xchacha20_round_trips_empty_plaintext() {
        let key = test_key();
        let nonce = test_nonce();
        let sealed = seal(AEAD_XCHACHA20_POLY1305, &key, &nonce, b"", b"").expect("seal empty");
        // Even empty plaintext produces a 16-byte tag.
        assert_eq!(sealed.len(), 16);
        let opened = open(AEAD_XCHACHA20_POLY1305, &key, &nonce, &sealed, b"").expect("open empty");
        assert!(opened.is_empty());
    }

    #[test]
    fn xchacha20_wrong_key_fails() {
        let key = test_key();
        let nonce = test_nonce();
        let wrong_key = vec![0x99u8; 32];
        let sealed = seal(AEAD_XCHACHA20_POLY1305, &key, &nonce, b"secret", b"").expect("seal");
        match open(AEAD_XCHACHA20_POLY1305, &wrong_key, &nonce, &sealed, b"") {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("open failed"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn xchacha20_wrong_aad_fails() {
        let key = test_key();
        let nonce = test_nonce();
        let sealed = seal(
            AEAD_XCHACHA20_POLY1305,
            &key,
            &nonce,
            b"secret",
            b"correct-aad",
        )
        .expect("seal");
        match open(AEAD_XCHACHA20_POLY1305, &key, &nonce, &sealed, b"wrong-aad") {
            Err(CoreError::Corrupt { .. }) => {}
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn xchacha20_tampered_ciphertext_fails() {
        let key = test_key();
        let nonce = test_nonce();
        let mut sealed = seal(AEAD_XCHACHA20_POLY1305, &key, &nonce, b"secret", b"").expect("seal");
        sealed[0] ^= 0xFF; // tamper
        match open(AEAD_XCHACHA20_POLY1305, &key, &nonce, &sealed, b"") {
            Err(CoreError::Corrupt { .. }) => {}
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_wrong_key_size() {
        match seal(
            AEAD_XCHACHA20_POLY1305,
            &[0u8; 16],
            &test_nonce(),
            b"data",
            b"",
        ) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("key size"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_wrong_nonce_size() {
        match seal(
            AEAD_XCHACHA20_POLY1305,
            &test_key(),
            &[0u8; 12],
            b"data",
            b"",
        ) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("nonce size"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_aead() {
        match seal(0xFF, &[], &[], b"data", b"") {
            Err(CoreError::UnsupportedFeature { .. }) => {}
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }
}
