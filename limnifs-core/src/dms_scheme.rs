//! DMS (distributed multi-share) scheme registry — maps DMS scheme ids
//! to algorithm metadata.
//!
//! Mirrors the [`crate::aead`] and [`crate::ec_scheme`] registries.
//! Each DMS policy section carries a scheme id; this registry
//! centralises the algorithm knowledge.
//!
//! ## Registered DMS schemes (v0.1)
//!
//! | Id | Name | Notes |
//! |---|---|---|
//! | 0x01 | Shamir | Shamir secret sharing over GF(2^8) |
//! | 0x02–0xFE | reserved | Future schemes |
//! | 0xFF | extended | Post-v1 descriptor |

/// DMS scheme id 0x01: Shamir secret sharing over GF(2^8).
pub const DMS_SCHEME_SHAMIR: u8 = 0x01;
/// Sentinel for extended DMS descriptor (post-v1).
pub const DMS_SCHEME_EXTENDED: u8 = 0xFF;

/// Metadata for a registered DMS scheme.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DmsSchemeInfo {
    pub id: u8,
    pub name: &'static str,
    pub galois_field: u16,
}

/// Look up the [`DmsSchemeInfo`] for `id`.
#[must_use]
pub const fn lookup(id: u8) -> Option<DmsSchemeInfo> {
    match id {
        DMS_SCHEME_SHAMIR => Some(DmsSchemeInfo {
            id: DMS_SCHEME_SHAMIR,
            name: "Shamir",
            galois_field: 256,
        }),
        _ => None,
    }
}

/// True iff `id` is a valid v0.1 DMS scheme id.
#[must_use]
pub const fn is_registered(id: u8) -> bool {
    matches!(id, DMS_SCHEME_SHAMIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shamir_metadata() {
        let info = lookup(DMS_SCHEME_SHAMIR).expect("Shamir registered");
        assert_eq!(info.name, "Shamir");
        assert_eq!(info.galois_field, 256);
    }

    #[test]
    fn reserved_returns_none() {
        assert!(lookup(0x00).is_none());
        assert!(lookup(0x02).is_none());
        assert!(lookup(0xFE).is_none());
    }

    #[test]
    fn extended_returns_none() {
        assert!(lookup(DMS_SCHEME_EXTENDED).is_none());
    }

    #[test]
    fn is_registered_check() {
        assert!(is_registered(DMS_SCHEME_SHAMIR));
        assert!(!is_registered(0x00));
        assert!(!is_registered(DMS_SCHEME_EXTENDED));
    }
}
