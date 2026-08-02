//! Encryption descriptor manifest section.
//!
//! Records which AEAD algorithm and key-wrap method was used,
//! along with the key-wrap parameters needed for decryption.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::cursor::ManifestCursor;
use crate::error::CoreError;

pub const ENCRYPTION_DESCRIPTOR_SECTION_VERSION: u8 = 1;
pub const KEY_WRAP_NONE: u8 = 0x00;
pub const KEY_WRAP_X25519_HKDF: u8 = 0x01;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncryptionDescriptor {
    pub version: u8,
    pub aead_id: u8,
    pub key_wrap_id: u8,
    pub key_wrap_params: KeyWrapParams,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyWrapParams {
    None,
    X25519Hkdf {
        recipient_pubkey: [u8; 32],
        ephemeral_pubkey: [u8; 32],
        wrapped_key_nonce: [u8; 24],
        wrapped_key: Vec<u8>,
    },
}

pub fn parse_encryption_descriptor(
    cursor: &mut ManifestCursor<'_>,
) -> Result<EncryptionDescriptor, CoreError> {
    let section_version = cursor.read_u8()?;
    if section_version != ENCRYPTION_DESCRIPTOR_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "encryption_descriptor version {section_version} (supported: {ENCRYPTION_DESCRIPTOR_SECTION_VERSION})"
            ),
        });
    }

    let aead_id = cursor.read_u8()?;
    let key_wrap_id = cursor.read_u8()?;
    let params_len = usize::from(cursor.read_u16_le()?);

    if cursor.remaining().len() < params_len {
        return Err(CoreError::TooShort {
            have: cursor.remaining().len(),
            need: params_len,
        });
    }

    let key_wrap_params = match key_wrap_id {
        KEY_WRAP_NONE => KeyWrapParams::None,
        KEY_WRAP_X25519_HKDF => {
            if params_len < 88 {
                return Err(CoreError::Corrupt {
                    reason: format!("X25519+HKDF params need 88+ bytes, got {params_len}"),
                });
            }
            let recipient_pubkey: [u8; 32] = cursor.read_n(32)?.try_into().unwrap();
            let ephemeral_pubkey: [u8; 32] = cursor.read_n(32)?.try_into().unwrap();
            let wrapped_key_nonce: [u8; 24] = cursor.read_n(24)?.try_into().unwrap();
            let wrapped_key_len = params_len.saturating_sub(88);
            let wrapped_key = cursor.read_n(wrapped_key_len)?.to_vec();
            KeyWrapParams::X25519Hkdf {
                recipient_pubkey,
                ephemeral_pubkey,
                wrapped_key_nonce,
                wrapped_key,
            }
        }
        other => {
            return Err(CoreError::UnsupportedFeature {
                feature: format!("key_wrap id {other:#04X}"),
            });
        }
    };

    Ok(EncryptionDescriptor {
        version: section_version,
        aead_id,
        key_wrap_id,
        key_wrap_params,
    })
}

pub fn encode_encryption_descriptor(desc: &EncryptionDescriptor, out: &mut Vec<u8>) {
    out.push(ENCRYPTION_DESCRIPTOR_SECTION_VERSION);
    out.push(desc.aead_id);
    out.push(desc.key_wrap_id);

    let mut params = Vec::new();
    match &desc.key_wrap_params {
        KeyWrapParams::None => {}
        KeyWrapParams::X25519Hkdf {
            recipient_pubkey,
            ephemeral_pubkey,
            wrapped_key_nonce,
            wrapped_key,
        } => {
            params.extend_from_slice(recipient_pubkey);
            params.extend_from_slice(ephemeral_pubkey);
            params.extend_from_slice(wrapped_key_nonce);
            params.extend_from_slice(wrapped_key);
        }
    }

    let params_len = u16::try_from(params.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&params_len.to_le_bytes());
    out.extend_from_slice(&params);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_none() {
        let original = EncryptionDescriptor {
            version: ENCRYPTION_DESCRIPTOR_SECTION_VERSION,
            aead_id: 0x00,
            key_wrap_id: KEY_WRAP_NONE,
            key_wrap_params: KeyWrapParams::None,
        };
        let mut encoded = Vec::new();
        encode_encryption_descriptor(&original, &mut encoded);

        let mut cursor = ManifestCursor::new(&encoded);
        let parsed = parse_encryption_descriptor(&mut cursor).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn round_trip_x25519() {
        let original = EncryptionDescriptor {
            version: ENCRYPTION_DESCRIPTOR_SECTION_VERSION,
            aead_id: 0x01,
            key_wrap_id: KEY_WRAP_X25519_HKDF,
            key_wrap_params: KeyWrapParams::X25519Hkdf {
                recipient_pubkey: [0x11; 32],
                ephemeral_pubkey: [0x22; 32],
                wrapped_key_nonce: [0x33; 24],
                wrapped_key: vec![0x44; 48],
            },
        };
        let mut encoded = Vec::new();
        encode_encryption_descriptor(&original, &mut encoded);

        let mut cursor = ManifestCursor::new(&encoded);
        let parsed = parse_encryption_descriptor(&mut cursor).expect("parse");
        assert_eq!(parsed, original);
    }
}
