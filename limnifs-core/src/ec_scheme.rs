//! EC (erasure coding) scheme registry — maps EC ids to algorithm
//! metadata.
//!
//! Each slab header carries an `ec_descriptor` byte. This registry
//! centralises the EC knowledge so the reader, writer, and future EC
//! module share a single source of truth.
//!
//! ## Registered EC schemes (v0.1)
//!
//! | Id | Name | Data shards | Parity shards | Notes |
//! |---|---|---|---|---|
//! | 0x00 | none | — | — | No erasure coding |
//! | 0x01 | Reed-Solomon GF(2^8) | configurable | configurable | Mandatory baseline |
//! | 0x02–0xFE | reserved | — | — | Future schemes |
//! | 0xFF | extended | — | — | Post-v1 descriptor |

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

/// EC id 0x00: no erasure coding.
pub const EC_NONE: u8 = 0x00;
/// EC id 0x01: Reed-Solomon over GF(2^8).
pub const EC_REED_SOLOMON_GF256: u8 = 0x01;
/// Sentinel for extended EC descriptor (post-v1).
pub const EC_EXTENDED: u8 = 0xFF;

/// Metadata for a registered EC scheme.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct EcInfo {
    /// The EC id (0x00–0xFE).
    pub id: u8,
    /// Human-readable name.
    pub name: &'static str,
    /// The Galois field this scheme operates over (0 if N/A).
    pub galois_field: u16,
}

impl EcInfo {
    /// True iff this slab actually uses erasure coding.
    #[must_use]
    pub const fn has_ec(self) -> bool {
        self.id != EC_NONE && self.id != EC_EXTENDED
    }
}

/// Look up the [`EcInfo`] for `id`.
#[must_use]
pub const fn lookup(id: u8) -> Option<EcInfo> {
    match id {
        EC_NONE => Some(EcInfo {
            id: EC_NONE,
            name: "none",
            galois_field: 0,
        }),
        EC_REED_SOLOMON_GF256 => Some(EcInfo {
            id: EC_REED_SOLOMON_GF256,
            name: "Reed-Solomon GF(2^8)",
            galois_field: 256,
        }),
        _ => None,
    }
}

/// True iff `id` is a valid v0.1 EC id.
#[must_use]
pub const fn is_registered(id: u8) -> bool {
    matches!(id, EC_NONE | EC_REED_SOLOMON_GF256)
}

/// All registered EC schemes in id order.
#[must_use]
pub fn registered() -> Vec<EcInfo> {
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
    fn none_has_no_ec() {
        let info = lookup(EC_NONE).expect("none registered");
        assert!(!info.has_ec());
    }

    #[test]
    fn reed_solomon_metadata() {
        let info = lookup(EC_REED_SOLOMON_GF256).expect("RS registered");
        assert_eq!(info.name, "Reed-Solomon GF(2^8)");
        assert_eq!(info.galois_field, 256);
        assert!(info.has_ec());
    }

    #[test]
    fn reserved_ids_return_none() {
        assert!(lookup(0x02).is_none());
        assert!(lookup(0xFE).is_none());
    }

    #[test]
    fn extended_returns_none() {
        assert!(lookup(EC_EXTENDED).is_none());
    }

    #[test]
    fn is_registered_for_known_ids() {
        assert!(is_registered(EC_NONE));
        assert!(is_registered(EC_REED_SOLOMON_GF256));
        assert!(!is_registered(0x02));
        assert!(!is_registered(EC_EXTENDED));
    }

    #[test]
    fn registered_lists_all_v0_1_schemes() {
        let all = registered();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, EC_NONE);
        assert_eq!(all[1].id, EC_REED_SOLOMON_GF256);
    }
}
