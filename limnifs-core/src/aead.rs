//! AEAD registry — maps AEAD ids to algorithm metadata.
//!
//! Each drop record and slab header carries an AEAD id in the
//! `representation` triple (codec, aead, ec) or the slab's
//! `crypto_hint` byte. This registry centralises the AEAD
//! knowledge so the reader, writer, and future crypto module share
//! a single source of truth.
//!
//! ## Registered AEADs (v0.1)
//!
//! | Id | Name | Key Size | Nonce Size | Tag Size | Notes |
//! |---|---|---|---|---|---|
//! | 0x00 | plaintext | 0 | 0 | 0 | No encryption |
//! | 0x01 | XChaCha20-Poly1305 | 32 | 24 | 16 | Mandatory baseline |
//! | 0x02–0xFE | reserved | — | — | — | Future AEADs |
//! | 0xFF | extended | — | — | — | Post-v1 descriptor |

/// AEAD id 0x00: plaintext (no encryption).
pub const AEAD_PLAINTEXT: u8 = 0x00;
/// AEAD id 0x01: XChaCha20-Poly1305 (mandatory baseline).
pub const AEAD_XCHACHA20_POLY1305: u8 = 0x01;
/// AEAD id 0x02: AES-256-GCM.
pub const AEAD_AES_256_GCM: u8 = 0x02;
/// AEAD id 0x03: AES-256-OCB (RFC 7253).
pub const AEAD_AES_256_OCB: u8 = 0x03;
/// Sentinel for extended AEAD descriptor (post-v1).
pub const AEAD_EXTENDED: u8 = 0xFF;

/// Metadata for a registered AEAD algorithm.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct AeadInfo {
    /// The AEAD id (0x00–0xFE).
    pub id: u8,
    /// Human-readable name (e.g. `"XChaCha20-Poly1305"`).
    pub name: &'static str,
    /// Key size in bytes (0 for plaintext).
    pub key_size: usize,
    /// Nonce size in bytes (0 for plaintext).
    pub nonce_size: usize,
    /// Authentication tag size in bytes (0 for plaintext).
    pub tag_size: usize,
}

impl AeadInfo {
    /// Total overhead added per sealed message: nonce + tag.
    /// Plaintext AEAD has zero overhead.
    #[must_use]
    pub const fn overhead(self) -> usize {
        self.nonce_size + self.tag_size
    }

    /// True iff this AEAD actually encrypts (i.e. not plaintext).
    #[must_use]
    pub const fn is_encrypted(self) -> bool {
        self.id != AEAD_PLAINTEXT && self.id != AEAD_EXTENDED
    }
}

/// Look up the [`AeadInfo`] for `id`. Returns `None` for reserved or
/// extended ids.
///
/// # Examples
///
/// ```
/// # use limnifs_core::aead::{lookup, AEAD_XCHACHA20_POLY1305};
/// let info = lookup(AEAD_XCHACHA20_POLY1305).expect("XChaCha20 is registered");
/// assert_eq!(info.name, "XChaCha20-Poly1305");
/// assert_eq!(info.key_size, 32);
/// ```
#[must_use]
pub const fn lookup(id: u8) -> Option<AeadInfo> {
    match id {
        AEAD_PLAINTEXT => Some(AeadInfo {
            id: AEAD_PLAINTEXT,
            name: "plaintext",
            key_size: 0,
            nonce_size: 0,
            tag_size: 0,
        }),
        AEAD_XCHACHA20_POLY1305 => Some(AeadInfo {
            id: AEAD_XCHACHA20_POLY1305,
            name: "XChaCha20-Poly1305",
            key_size: 32,
            nonce_size: 24,
            tag_size: 16,
        }),
        AEAD_AES_256_GCM => Some(AeadInfo {
            id: AEAD_AES_256_GCM,
            name: "AES-256-GCM",
            key_size: 32,
            nonce_size: 12,
            tag_size: 16,
        }),
        AEAD_AES_256_OCB => Some(AeadInfo {
            id: AEAD_AES_256_OCB,
            name: "AES-256-OCB",
            key_size: 32,
            nonce_size: 12,
            tag_size: 16,
        }),
        _ => None,
    }
}

/// Returns `true` iff `id` is a valid v0.2 AEAD id (0x00–0x03).
/// Extended (0xFF) and reserved (0x04–0xFE) are NOT valid for
/// registration in v0.2.
#[must_use]
pub const fn is_registered(id: u8) -> bool {
    matches!(
        id,
        AEAD_PLAINTEXT | AEAD_XCHACHA20_POLY1305 | AEAD_AES_256_GCM | AEAD_AES_256_OCB
    )
}

/// All registered AEADs in id order. Useful for listing available
/// algorithms in the CLI or docs.
#[must_use]
pub fn registered() -> Vec<AeadInfo> {
    let mut out = Vec::new();
    for id in 0x00..=0xFE {
        if let Some(info) = lookup(id) {
            out.push(info);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_has_zero_overhead() {
        let info = lookup(AEAD_PLAINTEXT).expect("plaintext registered");
        assert_eq!(info.overhead(), 0);
        assert!(!info.is_encrypted());
    }

    #[test]
    fn xchacha20_metadata() {
        let info = lookup(AEAD_XCHACHA20_POLY1305).expect("XChaCha20 registered");
        assert_eq!(info.name, "XChaCha20-Poly1305");
        assert_eq!(info.key_size, 32);
        assert_eq!(info.nonce_size, 24);
        assert_eq!(info.tag_size, 16);
        assert_eq!(info.overhead(), 40);
        assert!(info.is_encrypted());
    }

    #[test]
    fn aes_256_gcm_metadata() {
        let info = lookup(AEAD_AES_256_GCM).expect("AES-256-GCM registered");
        assert_eq!(info.key_size, 32);
        assert_eq!(info.nonce_size, 12);
        assert_eq!(info.tag_size, 16);
        assert_eq!(info.overhead(), 28);
        assert!(info.is_encrypted());
    }

    #[test]
    fn aes_256_ocb_metadata() {
        let info = lookup(AEAD_AES_256_OCB).expect("AES-256-OCB registered");
        assert_eq!(info.key_size, 32);
        assert_eq!(info.nonce_size, 12);
        assert_eq!(info.tag_size, 16);
        assert_eq!(info.overhead(), 28);
        assert!(info.is_encrypted());
    }

    #[test]
    fn reserved_ids_return_none() {
        assert!(lookup(0x04).is_none());
        assert!(lookup(0x7F).is_none());
        assert!(lookup(0xFE).is_none());
    }

    #[test]
    fn extended_returns_none() {
        assert!(lookup(AEAD_EXTENDED).is_none());
    }

    #[test]
    fn is_registered_for_known_ids() {
        assert!(is_registered(AEAD_PLAINTEXT));
        assert!(is_registered(AEAD_XCHACHA20_POLY1305));
        assert!(is_registered(AEAD_AES_256_GCM));
        assert!(is_registered(AEAD_AES_256_OCB));
        assert!(!is_registered(0x04));
        assert!(!is_registered(AEAD_EXTENDED));
    }

    #[test]
    fn registered_lists_all_v0_2_aeads() {
        let all = registered();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].id, AEAD_PLAINTEXT);
        assert_eq!(all[1].id, AEAD_XCHACHA20_POLY1305);
        assert_eq!(all[2].id, AEAD_AES_256_GCM);
        assert_eq!(all[3].id, AEAD_AES_256_OCB);
    }
}
