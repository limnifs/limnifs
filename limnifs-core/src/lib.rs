//! `LimniFS` core reader: manifest header parser.
//!
//! Source of truth: `limnifs/spec` §5.1 (Magic + format header).
//! The first 16 bytes of every `.lim` image manifest carry the magic,
//! the three independent per-layer version numbers, and a 4-byte
//! reserved field that MUST be zero. See `limnifs-format` for the
//! semantic types and magic constants.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use core::fmt;
use limnifs_format::MANIFEST_MAGIC;

pub use limnifs_format::MANIFEST_HEADER_LEN;

/// Error reading a manifest header.
///
/// Errors are surfaced verbatim to callers; the `limni` CLI maps them
/// to stable exit codes (see component 10-cli).
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CoreError {
    /// Fewer than 16 bytes available; cannot parse a header.
    TooShort { have: usize, need: usize },
    /// Magic bytes did not match `LMFS`.
    BadMagic { found: [u8; 4] },
    /// Header parsed, but a structural invariant was violated.
    Corrupt { reason: String },
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { have, need } => write!(
                f,
                "manifest header truncated: have {have} bytes, need {need}"
            ),
            Self::BadMagic { found } => write!(
                f,
                "bad manifest magic: expected LMFS ({:x?}), found {:?} ({:x?})",
                MANIFEST_MAGIC,
                core::str::from_utf8(found).unwrap_or("<non-utf8>"),
                found
            ),
            Self::Corrupt { reason } => write!(f, "manifest corrupt: {reason}"),
        }
    }
}

impl std::error::Error for CoreError {}

/// Parsed 16-byte manifest header (spec §5.1).
///
/// Field widths are fixed: magic (4) + three u16 LE versions (6) +
/// reserved (6). Reserved MUST be zero. The reserved width is what
/// reconciles the spec's "first 16 bytes" framing with the field table
/// (which otherwise sums to 14); a clarifying spec PR will land
/// alongside this code.
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

/// Parse a manifest header from a 16-byte slice (spec §5.1).
///
/// Validates magic, parses the three independent version fields, and
/// enforces the reserved-zero rule. Returns [`CoreError`] on any
/// structural problem. Successful parse does NOT validate versions or
/// feature flags — those are higher-layer concerns.
///
/// # Errors
///
/// - [`CoreError::TooShort`] if `bytes` is shorter than 16 bytes.
/// - [`CoreError::BadMagic`] if the first 4 bytes are not `LMFS`.
/// - [`CoreError::Corrupt`] if the reserved field is non-zero.
pub fn parse_manifest_header(bytes: &[u8]) -> Result<ManifestHeader, CoreError> {
    if bytes.len() < MANIFEST_HEADER_LEN {
        return Err(CoreError::TooShort {
            have: bytes.len(),
            need: MANIFEST_HEADER_LEN,
        });
    }
    let mut magic = [0u8; 4];
    magic.copy_from_slice(&bytes[..4]);
    if magic != MANIFEST_MAGIC {
        return Err(CoreError::BadMagic { found: magic });
    }
    let drop_store_version = u16::from_le_bytes([bytes[4], bytes[5]]);
    let metadata_version = u16::from_le_bytes([bytes[6], bytes[7]]);
    let manifest_version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if bytes[10..MANIFEST_HEADER_LEN].iter().any(|&b| b != 0) {
        return Err(CoreError::Corrupt {
            reason: format!(
                "reserved bytes 10..{} must be zero, found {:?}",
                MANIFEST_HEADER_LEN,
                &bytes[10..MANIFEST_HEADER_LEN]
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
        let header = parse_manifest_header(&bytes).expect("current header parses");
        assert_eq!(header, ManifestHeader::current());
        assert_eq!(header.drop_store_version, 1);
        assert_eq!(header.metadata_version, 1);
        assert_eq!(header.manifest_version, 1);
    }

    #[test]
    fn parses_zero_versions() {
        let mut bytes = current_header_bytes();
        bytes[4..10].fill(0);
        let header = parse_manifest_header(&bytes).expect("zero versions parse");
        assert_eq!(header.drop_store_version, 0);
        assert_eq!(header.metadata_version, 0);
        assert_eq!(header.manifest_version, 0);
    }

    #[test]
    fn rejects_short_buffer() {
        let short = [0u8; 15];
        match parse_manifest_header(&short) {
            Err(CoreError::TooShort { have, need }) => {
                assert_eq!(have, 15);
                assert_eq!(need, MANIFEST_HEADER_LEN);
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_buffer() {
        let empty: [u8; 0] = [];
        assert!(matches!(
            parse_manifest_header(&empty),
            Err(CoreError::TooShort { .. })
        ));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = current_header_bytes();
        bytes[0] = b'X';
        match parse_manifest_header(&bytes) {
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
        match parse_manifest_header(&bytes) {
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
        let header = parse_manifest_header(&bytes).expect("extra bytes are ignored");
        assert_eq!(header, ManifestHeader::current());
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

        let reparsed = parse_manifest_header(&bytes).expect("roundtrip");
        assert_eq!(reparsed, header);
    }

    #[test]
    fn error_display_messages_are_human_readable() {
        let short = CoreError::TooShort { have: 4, need: 16 };
        assert!(short.to_string().contains("16"));
        assert!(short.to_string().contains("truncated"));

        let bad = CoreError::BadMagic { found: *b"XXXX" };
        assert!(bad.to_string().contains("LMFS"));

        let corrupt = CoreError::Corrupt {
            reason: "test".into(),
        };
        assert!(corrupt.to_string().contains("test"));
    }
}
