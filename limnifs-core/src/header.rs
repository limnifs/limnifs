//! Manifest header (spec §5.1, `bit-level/35-manifest-header.md`).
//!
//! The first 16 bytes of every `.lim` image manifest. Magic `LMFS`,
//! three independent u16 LE version fields, and a 6-byte reserved
//! field that MUST be zero.

use crate::cursor::ManifestCursor;
use crate::error::CoreError;
use limnifs_format::{MANIFEST_HEADER_LEN, MANIFEST_MAGIC};

/// Parsed 16-byte manifest header.
///
/// Field widths are fixed: magic (4) + three u16 LE versions (6) +
/// reserved (6). Reserved MUST be zero. The reserved width reconciles
/// the spec's "first 16 bytes" framing with the field table (which
/// otherwise sums to 14) — see `bit-level/35-manifest-header.md` and
/// limnifs/spec#13.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ManifestHeader {
    pub drop_store_version: u16,
    pub metadata_version: u16,
    pub manifest_version: u16,
}

impl ManifestHeader {
    /// The currently implemented version of each layer. A header that
    /// names a future version is parseable structurally but flagged by
    /// higher layers (feature-flag policy, spec §18).
    pub const CURRENT_DROP_STORE_VERSION: u16 = 1;
    pub const CURRENT_METADATA_VERSION: u16 = 1;
    pub const CURRENT_MANIFEST_VERSION: u16 = 1;

    #[must_use]
    pub const fn new(
        drop_store_version: u16,
        metadata_version: u16,
        manifest_version: u16,
    ) -> Self {
        Self {
            drop_store_version,
            metadata_version,
            manifest_version,
        }
    }

    #[must_use]
    pub fn current() -> Self {
        Self::new(
            Self::CURRENT_DROP_STORE_VERSION,
            Self::CURRENT_METADATA_VERSION,
            Self::CURRENT_MANIFEST_VERSION,
        )
    }

    /// Serialise to the 16-byte wire form. Inverse of [`parse_manifest_header`].
    #[must_use]
    pub fn to_bytes(self) -> [u8; MANIFEST_HEADER_LEN] {
        let mut out = [0u8; MANIFEST_HEADER_LEN];
        out[..4].copy_from_slice(&MANIFEST_MAGIC);
        out[4..6].copy_from_slice(&self.drop_store_version.to_le_bytes());
        out[6..8].copy_from_slice(&self.metadata_version.to_le_bytes());
        out[8..10].copy_from_slice(&self.manifest_version.to_le_bytes());
        // bytes 10..16 are reserved, already zero
        out
    }
}

/// Parse the manifest header from the cursor's current position.
///
/// Advances the cursor by [`MANIFEST_HEADER_LEN`] bytes on success.
///
/// # Errors
///
/// - [`CoreError::BadMagic`] if the first 4 bytes are not `LMFS`.
/// - [`CoreError::TooShort`] if the cursor has fewer than 16 bytes.
/// - [`CoreError::Corrupt`] if the reserved field is non-zero.
///
/// Version validity (e.g. "is `manifest_version` 1 supported?") is a
/// higher-layer concern, handled by feature-flag policy (§18) and the
/// registry data (§14). The header parser only checks structural
/// invariants; it does not enforce version policy.
pub fn parse_manifest_header(cursor: &mut ManifestCursor<'_>) -> Result<ManifestHeader, CoreError> {
    let magic = cursor.read_magic()?;
    if magic != MANIFEST_MAGIC {
        return Err(CoreError::BadMagic { found: magic });
    }
    let drop_store_version = cursor.read_u16_le()?;
    let metadata_version = cursor.read_u16_le()?;
    let manifest_version = cursor.read_u16_le()?;
    let reserved = cursor.read_n(6)?;
    if reserved.iter().any(|&b| b != 0) {
        return Err(CoreError::Corrupt {
            reason: format!(
                "reserved bytes 10..{MANIFEST_HEADER_LEN} must be zero, found {reserved:?}"
            ),
        });
    }
    Ok(ManifestHeader {
        drop_store_version,
        metadata_version,
        manifest_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current_header_bytes() -> [u8; MANIFEST_HEADER_LEN] {
        let mut bytes = [0u8; MANIFEST_HEADER_LEN];
        bytes[..4].copy_from_slice(b"LMFS");
        bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&1u16.to_le_bytes());
        bytes[8..10].copy_from_slice(&1u16.to_le_bytes());
        bytes
    }

    #[test]
    fn parses_current_header() {
        let bytes = current_header_bytes();
        let mut cursor = ManifestCursor::new(&bytes);
        let header = parse_manifest_header(&mut cursor).expect("current header parses");
        assert_eq!(header, ManifestHeader::current());
        assert_eq!(cursor.position(), MANIFEST_HEADER_LEN);
    }

    #[test]
    fn parses_zero_versions() {
        let mut bytes = current_header_bytes();
        bytes[4..10].fill(0);
        let mut cursor = ManifestCursor::new(&bytes);
        let header = parse_manifest_header(&mut cursor).expect("zero versions parse");
        assert_eq!(header.drop_store_version, 0);
        assert_eq!(header.metadata_version, 0);
        assert_eq!(header.manifest_version, 0);
    }

    #[test]
    fn rejects_short_buffer() {
        // Valid magic + valid versions, but the reserved field is
        // truncated. Cursor's read_n(6) returns TooShort when it
        // reaches the missing bytes.
        let mut short = [0u8; 12];
        short[..4].copy_from_slice(b"LMFS");
        short[4..6].copy_from_slice(&1u16.to_le_bytes());
        short[6..8].copy_from_slice(&1u16.to_le_bytes());
        short[8..10].copy_from_slice(&1u16.to_le_bytes());
        let mut cursor = ManifestCursor::new(&short);
        match parse_manifest_header(&mut cursor) {
            Err(CoreError::TooShort { have, need }) => {
                assert_eq!(have, 2);
                assert_eq!(need, 6);
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_buffer() {
        let empty: [u8; 0] = [];
        let mut cursor = ManifestCursor::new(&empty);
        assert!(matches!(
            parse_manifest_header(&mut cursor),
            Err(CoreError::TooShort { .. })
        ));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = current_header_bytes();
        bytes[0] = b'X';
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_manifest_header(&mut cursor) {
            Err(CoreError::BadMagic { found }) => {
                assert_eq!(found, *b"XMFS");
            }
            other => panic!("expected BadMagic, got {other:?}"),
        }
    }

    #[test]
    fn rejects_nonzero_reserved() {
        let mut bytes = current_header_bytes();
        bytes[13] = 0x01;
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_manifest_header(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("reserved"), "reason was: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn accepts_extra_bytes_after_header() {
        let mut bytes = vec![0u8; MANIFEST_HEADER_LEN + 32];
        bytes[..MANIFEST_HEADER_LEN].copy_from_slice(&current_header_bytes());
        let mut cursor = ManifestCursor::new(&bytes);
        let header = parse_manifest_header(&mut cursor).expect("extra bytes are ignored");
        assert_eq!(header, ManifestHeader::current());
        assert_eq!(cursor.position(), MANIFEST_HEADER_LEN);
        assert_eq!(cursor.remaining_len(), 32);
    }

    #[test]
    fn round_trip_to_bytes() {
        let header = ManifestHeader::new(7, 11, 13);
        let bytes = header.to_bytes();
        assert_eq!(&bytes[..4], b"LMFS");
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 7);
        assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), 11);
        assert_eq!(u16::from_le_bytes([bytes[8], bytes[9]]), 13);
        assert_eq!(&bytes[10..MANIFEST_HEADER_LEN], &[0, 0, 0, 0, 0, 0]);

        let mut cursor = ManifestCursor::new(&bytes);
        let reparsed = parse_manifest_header(&mut cursor).expect("roundtrip");
        assert_eq!(reparsed, header);
    }
}
