//! Slab header (spec §3.2, `bit-level/30-slab-header.md`).
//!
//! The 56-byte fixed-size prefix at offset 0 of every slab in the
//! drop store. Magic `LIM1`, u16 LE `format_version`, `SlabId`
//! (ordinal + hash), u64 LE `total_length`, u8 `ec_descriptor`, u8
//! `crypto_hint`.

use crate::cursor::ManifestCursor;
use crate::error::CoreError;
use limnifs_format::{SlabId, SLAB_MAGIC};

/// Default slab-size ceiling per [§3.1](https://github.com/limnifs/spec/blob/main/wire-format/21-drop-store.md):
/// slabs MUST be ≤ 64 MiB unless the manifest overrides.
pub const DEFAULT_SLAB_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Current slab layout version (matches `format_version` byte 4..6).
pub const SLAB_FORMAT_VERSION: u16 = 1;

/// Width of the fixed slab header.
pub const SLAB_HEADER_LEN: usize = 56;

/// Sentinel value for `ec_descriptor` indicating an extended (post-v1)
/// descriptor follows. Readers in v1 reject with `UnsupportedFeature`.
pub const EC_DESCRIPTOR_EXTENDED: u8 = 0xFF;

/// Sentinel value for `crypto_hint` indicating an extended (post-v1)
/// hint follows. Readers in v1 reject with `UnsupportedFeature`.
pub const CRYPTO_HINT_EXTENDED: u8 = 0xFF;

/// Parsed slab header.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SlabHeader {
    pub format_version: u16,
    pub slab_id: SlabId,
    pub total_length: u64,
    pub ec_descriptor: u8,
    pub crypto_hint: u8,
}

impl SlabHeader {
    /// True iff this slab carries Reed-Solomon parity shards.
    #[must_use]
    pub const fn has_erasure_coding(self) -> bool {
        self.ec_descriptor != 0x00 && self.ec_descriptor != EC_DESCRIPTOR_EXTENDED
    }

    /// True iff this slab's payload is sealed with an AEAD.
    #[must_use]
    pub const fn is_sealed(self) -> bool {
        self.crypto_hint != 0x00 && self.crypto_hint != CRYPTO_HINT_EXTENDED
    }
}

/// Parse a slab header from the cursor's current position.
///
/// Advances the cursor by [`SLAB_HEADER_LEN`] bytes on success.
/// Validates magic, format version, `total_length` floor (must be at
/// least the header width), and rejects extended descriptors/hints
/// (`0xFF`) with [`CoreError::UnsupportedFeature`].
///
/// # Errors
///
/// - [`CoreError::BadMagic`] if the first 4 bytes are not `LIM1`.
/// - [`CoreError::TooShort`] if the cursor has fewer than 56 bytes.
/// - [`CoreError::UnsupportedFeature`] if `format_version` is not 1,
///   or if `ec_descriptor == 0xFF`, or if `crypto_hint == 0xFF`.
/// - [`CoreError::Corrupt`] if `total_length < SLAB_HEADER_LEN`.
pub fn parse_slab_header(cursor: &mut ManifestCursor<'_>) -> Result<SlabHeader, CoreError> {
    parse_slab_header_with_ceiling(cursor, DEFAULT_SLAB_MAX_BYTES)
}

/// Same as [`parse_slab_header`] but lets the caller supply a
/// `max_total_length` overriding the 64 MiB default. Used by readers
/// that have parsed a manifest with a non-default slab-size parameter.
///
/// # Errors
///
/// Inherits all errors from [`parse_slab_header`], and additionally
/// returns [`CoreError::Corrupt`] when `total_length > max_total_length`.
pub fn parse_slab_header_with_ceiling(
    cursor: &mut ManifestCursor<'_>,
    max_total_length: u64,
) -> Result<SlabHeader, CoreError> {
    let magic = cursor.read_magic()?;
    if magic != SLAB_MAGIC {
        return Err(CoreError::BadMagic { found: magic });
    }
    let format_version = cursor.read_u16_le()?;
    if format_version != SLAB_FORMAT_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "slab format_version {format_version} (supported: {SLAB_FORMAT_VERSION})"
            ),
        });
    }
    let ordinal = cursor.read_u64_le()?;
    let hash = cursor.read_n(32)?;
    let mut hash_array = [0u8; 32];
    hash_array.copy_from_slice(hash);
    let slab_id = SlabId::new(ordinal, hash_array);
    let total_length = cursor.read_u64_le()?;
    if total_length < u64::try_from(SLAB_HEADER_LEN).unwrap_or(u64::MAX) {
        return Err(CoreError::Corrupt {
            reason: format!(
                "slab total_length {total_length} is less than header width {SLAB_HEADER_LEN}"
            ),
        });
    }
    if total_length > max_total_length {
        return Err(CoreError::Corrupt {
            reason: format!(
                "slab total_length {total_length} exceeds configured ceiling {max_total_length}"
            ),
        });
    }
    let ec_descriptor = cursor.read_u8()?;
    if ec_descriptor == EC_DESCRIPTOR_EXTENDED {
        return Err(CoreError::UnsupportedFeature {
            feature: "slab ec_descriptor 0xFF (extended descriptor, post-v1)".into(),
        });
    }
    let crypto_hint = cursor.read_u8()?;
    if crypto_hint == CRYPTO_HINT_EXTENDED {
        return Err(CoreError::UnsupportedFeature {
            feature: "slab crypto_hint 0xFF (extended hint, post-v1)".into(),
        });
    }
    Ok(SlabHeader {
        format_version,
        slab_id,
        total_length,
        ec_descriptor,
        crypto_hint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_slab_header_bytes(
        format_version: u16,
        slab_id: SlabId,
        total_length: u64,
        ec_descriptor: u8,
        crypto_hint: u8,
    ) -> [u8; SLAB_HEADER_LEN] {
        let mut bytes = [0u8; SLAB_HEADER_LEN];
        bytes[..4].copy_from_slice(&SLAB_MAGIC);
        bytes[4..6].copy_from_slice(&format_version.to_le_bytes());
        let slab_id_bytes = slab_id.to_bytes();
        bytes[6..46].copy_from_slice(&slab_id_bytes);
        bytes[46..54].copy_from_slice(&total_length.to_le_bytes());
        bytes[54] = ec_descriptor;
        bytes[55] = crypto_hint;
        bytes
    }

    fn sample_slab_id() -> SlabId {
        SlabId::new(7, [0xAA; 32])
    }

    #[test]
    fn parses_current_plaintext_header() {
        let bytes = make_slab_header_bytes(1, sample_slab_id(), 4096, 0x00, 0x00);
        let mut cursor = ManifestCursor::new(&bytes);
        let header = parse_slab_header(&mut cursor).expect("current header parses");
        assert_eq!(header.format_version, 1);
        assert_eq!(header.slab_id, sample_slab_id());
        assert_eq!(header.total_length, 4096);
        assert_eq!(header.ec_descriptor, 0x00);
        assert_eq!(header.crypto_hint, 0x00);
        assert!(!header.has_erasure_coding());
        assert!(!header.is_sealed());
        assert_eq!(cursor.position(), SLAB_HEADER_LEN);
    }

    #[test]
    fn parses_ec_enabled_sealed_header() {
        let bytes = make_slab_header_bytes(1, sample_slab_id(), 16_384, 0x01, 0x01);
        let mut cursor = ManifestCursor::new(&bytes);
        let header = parse_slab_header(&mut cursor).expect("EC + sealed parses");
        assert!(header.has_erasure_coding());
        assert!(header.is_sealed());
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = make_slab_header_bytes(1, sample_slab_id(), 4096, 0, 0);
        bytes[0] = b'X';
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_slab_header(&mut cursor) {
            Err(CoreError::BadMagic { found }) => {
                assert_eq!(found, [b'X', b'I', b'M', b'1']);
            }
            other => panic!("expected BadMagic, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_format_version() {
        let bytes = make_slab_header_bytes(7, sample_slab_id(), 4096, 0, 0);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_slab_header(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("format_version 7"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_total_length_below_header_width() {
        let bytes = make_slab_header_bytes(1, sample_slab_id(), 32, 0, 0);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_slab_header(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("header width"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_total_length_above_default_ceiling() {
        let bytes = make_slab_header_bytes(1, sample_slab_id(), DEFAULT_SLAB_MAX_BYTES + 1, 0, 0);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_slab_header(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("ceiling"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn custom_ceiling_accepts_oversized_slab() {
        let bytes = make_slab_header_bytes(1, sample_slab_id(), DEFAULT_SLAB_MAX_BYTES + 1, 0, 0);
        let mut cursor = ManifestCursor::new(&bytes);
        let header = parse_slab_header_with_ceiling(&mut cursor, DEFAULT_SLAB_MAX_BYTES * 2)
            .expect("custom ceiling accepts");
        assert_eq!(header.total_length, DEFAULT_SLAB_MAX_BYTES + 1);
    }

    #[test]
    fn rejects_extended_ec_descriptor() {
        let bytes = make_slab_header_bytes(1, sample_slab_id(), 4096, 0xFF, 0);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_slab_header(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("ec_descriptor"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_extended_crypto_hint() {
        let bytes = make_slab_header_bytes(1, sample_slab_id(), 4096, 0, 0xFF);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_slab_header(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("crypto_hint"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_truncated_header() {
        // Valid magic + valid format_version, but the slab_id and
        // remaining fields do not fit. Cursor returns TooShort when
        // it reaches the missing bytes.
        let mut bytes = [0u8; 50];
        bytes[..4].copy_from_slice(&SLAB_MAGIC);
        bytes[4..6].copy_from_slice(&SLAB_FORMAT_VERSION.to_le_bytes());
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_slab_header(&mut cursor) {
            Err(CoreError::TooShort { .. }) => {}
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_via_constructor() {
        let header = SlabHeader {
            format_version: SLAB_FORMAT_VERSION,
            slab_id: sample_slab_id(),
            total_length: 8192,
            ec_descriptor: 0x01,
            crypto_hint: 0x01,
        };
        let bytes = make_slab_header_bytes(
            header.format_version,
            header.slab_id,
            header.total_length,
            header.ec_descriptor,
            header.crypto_hint,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let reparsed = parse_slab_header(&mut cursor).expect("roundtrip");
        assert_eq!(reparsed, header);
    }
}
