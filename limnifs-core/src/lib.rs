//! `LimniFS` core reader: manifest header + feature flags parser.
//!
//! Source of truth: `limnifs/spec` §5.1 (Magic + format header),
//! §5.2 (Feature flags), and `bit-level/35-manifest-header.md` +
//! `bit-level/36-feature-flags.md` for byte-level layouts.
//! See `limnifs-format` for the semantic types and magic constants.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use core::fmt;
use limnifs_format::MANIFEST_MAGIC;

pub use limnifs_format::MANIFEST_HEADER_LEN;

/// Error reading a manifest header or section.
///
/// Errors are surfaced verbatim to callers; the `limni` CLI maps them
/// to stable exit codes (see component 10-cli).
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CoreError {
    /// Fewer than the required bytes available.
    TooShort { have: usize, need: usize },
    /// Magic bytes did not match `LMFS`.
    BadMagic { found: [u8; 4] },
    /// A structural invariant was violated (nonzero reserved, bad
    /// section version, duplicate flag, out-of-range value, etc.).
    Corrupt { reason: String },
    /// The image uses a feature the reader does not implement.
    /// Carries the flag id (for feature flags) or the section version
    /// (for unknown section layouts); callers disambiguate via context.
    UnsupportedFeature { feature: String },
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
            Self::UnsupportedFeature { feature } => {
                write!(f, "unsupported feature: {feature}")
            }
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

/// Current layout version of the feature flags section
/// (`bit-level/36-feature-flags.md`).
pub const FEATURE_FLAGS_SECTION_VERSION: u8 = 1;

/// Width of the fixed-size prefix of the feature flags section
/// (version byte + u32 LE entry count).
const FEATURE_FLAGS_PREFIX_LEN: usize = 5;

/// Width of a single feature flag entry (`u16 LE` flag id + `u8` required).
const FEATURE_FLAG_ENTRY_LEN: usize = 3;

/// One row of the manifest's feature flags section (spec §5.2,
/// `bit-level/36-feature-flags.md`).
///
/// `flag_id` references the feature-flag registry (spec §14).
/// `required` reflects the wire byte: a required flag the reader does
/// not know causes `UnsupportedFeature`; an optional flag is silently
/// ignored (spec §18).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct FeatureFlag {
    pub flag_id: u16,
    pub required: bool,
}

/// Parsed feature flags section.
///
/// `entries` is in wire order (declaration order in the manifest).
/// Duplicate flag ids are rejected at parse time per
/// `bit-level/36-feature-flags.md` validation rule 6.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct FeatureFlags {
    pub entries: Vec<FeatureFlag>,
}

impl FeatureFlags {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Look up the entry for `flag_id`, if present.
    #[must_use]
    pub fn get(&self, flag_id: u16) -> Option<FeatureFlag> {
        self.entries
            .iter()
            .copied()
            .find(|entry| entry.flag_id == flag_id)
    }

    /// True iff `flag_id` is declared with `required = true`.
    #[must_use]
    pub fn is_required(&self, flag_id: u16) -> bool {
        self.get(flag_id).is_some_and(|entry| entry.required)
    }
}

/// Parse the feature flags section starting at byte `offset` of `bytes`.
///
/// Returns the parsed flags and the number of bytes consumed (the
/// section's total width: `5 + 3 × N`). Callers advance the cursor by
/// the returned count to reach the next section.
///
/// # Errors
///
/// - [`CoreError::TooShort`] if `bytes[offset..]` is shorter than the
///   fixed prefix or the declared payload.
/// - [`CoreError::UnsupportedFeature`] if the section version is not
///   [`FEATURE_FLAGS_SECTION_VERSION`].
/// - [`CoreError::Corrupt`] for: zero `flag_id`, `required` byte
///   outside `{0, 1}`, duplicate `flag_id`.
pub fn parse_feature_flags_section(
    bytes: &[u8],
    offset: usize,
) -> Result<(FeatureFlags, usize), CoreError> {
    if offset
        .checked_add(FEATURE_FLAGS_PREFIX_LEN)
        .map_or(true, |end| end > bytes.len())
    {
        return Err(CoreError::TooShort {
            have: bytes.len().saturating_sub(offset),
            need: FEATURE_FLAGS_PREFIX_LEN,
        });
    }
    let section_version = bytes[offset];
    if section_version != FEATURE_FLAGS_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "feature_flags section version {section_version} (supported: {FEATURE_FLAGS_SECTION_VERSION})"
            ),
        });
    }
    let entry_count = u32::from_le_bytes([
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
    ]);
    let entry_count = usize::try_from(entry_count).map_err(|_| CoreError::Corrupt {
        reason: format!("feature_flags entry count {entry_count} exceeds usize"),
    })?;
    let payload_len = entry_count
        .checked_mul(FEATURE_FLAG_ENTRY_LEN)
        .and_then(|product| product.checked_add(FEATURE_FLAGS_PREFIX_LEN))
        .ok_or_else(|| CoreError::Corrupt {
            reason: format!("feature_flags entry count {entry_count} overflows section size"),
        })?;
    if offset
        .checked_add(payload_len)
        .map_or(true, |end| end > bytes.len())
    {
        return Err(CoreError::TooShort {
            have: bytes.len().saturating_sub(offset),
            need: payload_len,
        });
    }
    let mut entries = Vec::with_capacity(entry_count);
    for index in 0..entry_count {
        let entry_offset = offset + FEATURE_FLAGS_PREFIX_LEN + index * FEATURE_FLAG_ENTRY_LEN;
        let flag_id = u16::from_le_bytes([bytes[entry_offset], bytes[entry_offset + 1]]);
        if flag_id == 0 {
            return Err(CoreError::Corrupt {
                reason: format!("feature_flags entry {index}: flag_id 0x0000 is reserved"),
            });
        }
        let required_byte = bytes[entry_offset + 2];
        let required = match required_byte {
            0x00 => false,
            0x01 => true,
            other => {
                return Err(CoreError::Corrupt {
                    reason: format!(
                        "feature_flags entry {index}: required byte must be 0x00 or 0x01, got 0x{other:02X}"
                    ),
                });
            }
        };
        if entries
            .iter()
            .any(|existing: &FeatureFlag| existing.flag_id == flag_id)
        {
            return Err(CoreError::Corrupt {
                reason: format!("feature_flags entry {index}: duplicate flag_id 0x{flag_id:04X}"),
            });
        }
        entries.push(FeatureFlag { flag_id, required });
    }
    Ok((FeatureFlags { entries }, payload_len))
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

        let unsupported = CoreError::UnsupportedFeature {
            feature: "feature_flags section version 7".into(),
        };
        let s = unsupported.to_string();
        assert!(s.contains("unsupported"), "got: {s}");
        assert!(s.contains("version 7"));
    }

    fn make_flags_bytes(version: u8, entries: &[(u16, u8)]) -> Vec<u8> {
        let mut bytes =
            Vec::with_capacity(FEATURE_FLAGS_PREFIX_LEN + entries.len() * FEATURE_FLAG_ENTRY_LEN);
        bytes.push(version);
        let count = u32::try_from(entries.len()).expect("test entries fit in u32");
        bytes.extend_from_slice(&count.to_le_bytes());
        for (flag_id, required) in entries {
            bytes.extend_from_slice(&flag_id.to_le_bytes());
            bytes.push(*required);
        }
        bytes
    }

    #[test]
    fn feature_flags_parses_empty_section() {
        let bytes = make_flags_bytes(FEATURE_FLAGS_SECTION_VERSION, &[]);
        let (flags, consumed) =
            parse_feature_flags_section(&bytes, 0).expect("empty section parses");
        assert!(flags.is_empty());
        assert_eq!(flags.len(), 0);
        assert_eq!(consumed, FEATURE_FLAGS_PREFIX_LEN);
    }

    #[test]
    fn feature_flags_parses_single_required_ec_flag() {
        let bytes = make_flags_bytes(FEATURE_FLAGS_SECTION_VERSION, &[(0x0001, 0x01)]);
        let (flags, consumed) = parse_feature_flags_section(&bytes, 0).expect("single flag parses");
        assert_eq!(consumed, FEATURE_FLAGS_PREFIX_LEN + FEATURE_FLAG_ENTRY_LEN);
        assert_eq!(flags.len(), 1);
        assert_eq!(
            flags.entries[0],
            FeatureFlag {
                flag_id: 0x0001,
                required: true,
            }
        );
        assert!(flags.is_required(0x0001));
        assert!(!flags.is_required(0x0002));
    }

    #[test]
    fn feature_flags_parses_mixed_required_and_optional() {
        let bytes = make_flags_bytes(
            FEATURE_FLAGS_SECTION_VERSION,
            &[(0x0001, 0x01), (0x0012, 0x00), (0x0020, 0x01)],
        );
        let (flags, consumed) = parse_feature_flags_section(&bytes, 0).expect("mixed flags parse");
        assert_eq!(
            consumed,
            FEATURE_FLAGS_PREFIX_LEN + 3 * FEATURE_FLAG_ENTRY_LEN
        );
        assert_eq!(flags.len(), 3);
        assert!(flags.is_required(0x0001));
        assert!(!flags.is_required(0x0012));
        assert!(flags.is_required(0x0020));
        assert_eq!(flags.get(0x0012).unwrap().flag_id, 0x0012);
    }

    #[test]
    fn feature_flags_parses_at_nonzero_offset() {
        let mut bytes = vec![0u8; 16];
        let section = make_flags_bytes(FEATURE_FLAGS_SECTION_VERSION, &[(0x0001, 0x01)]);
        bytes.extend_from_slice(&section);
        let (flags, consumed) = parse_feature_flags_section(&bytes, 16).expect("offset parse");
        assert_eq!(consumed, section.len());
        assert_eq!(flags.len(), 1);
    }

    #[test]
    fn feature_flags_rejects_unknown_section_version() {
        let bytes = make_flags_bytes(7, &[]);
        match parse_feature_flags_section(&bytes, 0) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("version 7"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn feature_flags_rejects_short_prefix() {
        let bytes = [0u8; 4];
        match parse_feature_flags_section(&bytes, 0) {
            Err(CoreError::TooShort { have, need }) => {
                assert_eq!(have, 4);
                assert_eq!(need, FEATURE_FLAGS_PREFIX_LEN);
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn feature_flags_rejects_truncated_entries() {
        // Declare count = 10 but only provide one entry. Parser must
        // detect the shortfall when checking 5 + 3 * 10 > available.
        let mut bytes = Vec::new();
        bytes.push(FEATURE_FLAGS_SECTION_VERSION);
        bytes.extend_from_slice(&10u32.to_le_bytes());
        bytes.extend_from_slice(&[0x01, 0x00, 0x01]); // only one entry provided
        match parse_feature_flags_section(&bytes, 0) {
            Err(CoreError::TooShort { have, need }) => {
                assert_eq!(have, bytes.len());
                assert_eq!(need, FEATURE_FLAGS_PREFIX_LEN + 10 * FEATURE_FLAG_ENTRY_LEN);
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn feature_flags_rejects_zero_flag_id() {
        let bytes = make_flags_bytes(FEATURE_FLAGS_SECTION_VERSION, &[(0x0000, 0x01)]);
        match parse_feature_flags_section(&bytes, 0) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("0x0000"), "got: {reason}");
                assert!(reason.contains("reserved"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn feature_flags_rejects_bad_required_byte() {
        let bytes = make_flags_bytes(FEATURE_FLAGS_SECTION_VERSION, &[(0x0001, 0x05)]);
        match parse_feature_flags_section(&bytes, 0) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("required"), "got: {reason}");
                assert!(reason.contains("0x05"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn feature_flags_rejects_duplicate_flag_id() {
        let bytes = make_flags_bytes(
            FEATURE_FLAGS_SECTION_VERSION,
            &[(0x0001, 0x01), (0x0001, 0x00)],
        );
        match parse_feature_flags_section(&bytes, 0) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("duplicate"), "got: {reason}");
                assert!(reason.contains("0x0001"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn feature_flags_round_trip_all_standard_flags() {
        let standard: &[(u16, u8)] = &[
            (0x0001, 0x01),
            (0x0002, 0x00),
            (0x0010, 0x00),
            (0x0011, 0x00),
            (0x0012, 0x01),
            (0x0013, 0x00),
            (0x0014, 0x00),
            (0x0020, 0x01),
            (0x0021, 0x00),
            (0x0022, 0x00),
            (0x0100, 0x00),
            (0x0101, 0x00),
        ];
        let bytes = make_flags_bytes(FEATURE_FLAGS_SECTION_VERSION, standard);
        let (flags, consumed) = parse_feature_flags_section(&bytes, 0).expect("all standard parse");
        assert_eq!(flags.len(), standard.len());
        assert_eq!(
            consumed,
            FEATURE_FLAGS_PREFIX_LEN + standard.len() * FEATURE_FLAG_ENTRY_LEN
        );
        for (entry, (flag_id, required)) in flags.entries.iter().zip(standard.iter()) {
            assert_eq!(entry.flag_id, *flag_id);
            assert_eq!(entry.required, *required != 0);
        }
    }
}
