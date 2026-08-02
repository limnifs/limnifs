//! Feature flags section (spec §5.2, `bit-level/36-feature-flags.md`).
//!
//! One row of `(flag_id, required)` per optional feature the image
//! relies on. Readers apply the unknown-flag policy (§18): an unknown
//! REQUIRED flag fails the read; an unknown optional flag is silently
//! ignored.

use crate::cursor::ManifestCursor;
use crate::error::CoreError;

/// Current layout version of this section.
pub const FEATURE_FLAGS_SECTION_VERSION: u8 = 1;

/// Width of the fixed prefix (version byte + u32 LE entry count).
/// Exposed so callers can size buffers and verify cursor advancement.
pub const PREFIX_LEN: usize = 5;

/// Width of a single entry (`u16 LE` flag id + `u8` required).
pub const ENTRY_LEN: usize = 3;

/// One feature-flag entry from the manifest.
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

/// Parse the feature flags section from the cursor's current position.
///
/// Advances the cursor by the section's total width
/// (`PREFIX_LEN + ENTRY_LEN × entry_count`) on success.
///
/// # Errors
///
/// - [`CoreError::UnsupportedFeature`] if the section version is not
///   [`FEATURE_FLAGS_SECTION_VERSION`].
/// - [`CoreError::Corrupt`] if `entry_count` exceeds `usize`, if any
///   `flag_id` is `0x0000`, if any `required` byte is not in
///   `{0x00, 0x01}`, or if a `flag_id` is declared more than once.
/// - [`CoreError::TooShort`] if the cursor has fewer bytes than the
///   section declares.
pub fn parse_feature_flags_section(
    cursor: &mut ManifestCursor<'_>,
) -> Result<FeatureFlags, CoreError> {
    let section_version = cursor.read_u8()?;
    if section_version != FEATURE_FLAGS_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "feature_flags section version {section_version} (supported: {FEATURE_FLAGS_SECTION_VERSION})"
            ),
        });
    }
    let raw_count = cursor.read_u32_le()?;
    let entry_count = usize::try_from(raw_count).map_err(|_| CoreError::Corrupt {
        reason: format!("feature_flags entry count {raw_count} exceeds usize"),
    })?;
    // Verify the declared count fits the remaining bytes BEFORE we
    // call Vec::with_capacity. Without this, a malicious header with
    // entry_count = u32::MAX would ask the allocator for ~12 GB and
    // abort the reader (DoS).
    let payload_size = entry_count
        .checked_mul(ENTRY_LEN)
        .ok_or_else(|| CoreError::Corrupt {
            reason: format!("feature_flags entry count {entry_count} overflows section size"),
        })?;
    if cursor.remaining_len() < payload_size {
        return Err(CoreError::TooShort {
            have: cursor.remaining_len(),
            need: payload_size,
        });
    }
    let mut entries = Vec::with_capacity(entry_count);
    for index in 0..entry_count {
        let flag_id = cursor.read_u16_le()?;
        if flag_id == 0 {
            return Err(CoreError::Corrupt {
                reason: format!("feature_flags entry {index}: flag_id 0x0000 is reserved"),
            });
        }
        let required_byte = cursor.read_u8()?;
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
    Ok(FeatureFlags { entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_flags_bytes(version: u8, entries: &[(u16, u8)]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PREFIX_LEN + entries.len() * ENTRY_LEN);
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
    fn parses_empty_section() {
        let bytes = make_flags_bytes(FEATURE_FLAGS_SECTION_VERSION, &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        let flags = parse_feature_flags_section(&mut cursor).expect("empty section parses");
        assert!(flags.is_empty());
        assert_eq!(flags.len(), 0);
        assert_eq!(cursor.position(), PREFIX_LEN);
    }

    #[test]
    fn parses_single_required_ec_flag() {
        let bytes = make_flags_bytes(FEATURE_FLAGS_SECTION_VERSION, &[(0x0001, 0x01)]);
        let mut cursor = ManifestCursor::new(&bytes);
        let flags = parse_feature_flags_section(&mut cursor).expect("single flag parses");
        assert_eq!(cursor.position(), PREFIX_LEN + ENTRY_LEN);
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
    fn parses_mixed_required_and_optional() {
        let bytes = make_flags_bytes(
            FEATURE_FLAGS_SECTION_VERSION,
            &[(0x0001, 0x01), (0x0012, 0x00), (0x0020, 0x01)],
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let flags = parse_feature_flags_section(&mut cursor).expect("mixed flags parse");
        assert_eq!(cursor.position(), PREFIX_LEN + 3 * ENTRY_LEN);
        assert_eq!(flags.len(), 3);
        assert!(flags.is_required(0x0001));
        assert!(!flags.is_required(0x0012));
        assert!(flags.is_required(0x0020));
        assert_eq!(flags.get(0x0012).unwrap().flag_id, 0x0012);
    }

    #[test]
    fn parses_after_a_header() {
        let mut bytes = vec![0u8; 16];
        bytes[..4].copy_from_slice(b"LMFS");
        bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&1u16.to_le_bytes());
        bytes[8..10].copy_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&make_flags_bytes(
            FEATURE_FLAGS_SECTION_VERSION,
            &[(0x0001, 0x01)],
        ));
        let mut cursor = ManifestCursor::new(&bytes);
        cursor.skip(16).unwrap();
        let flags = parse_feature_flags_section(&mut cursor).expect("after header");
        assert_eq!(flags.len(), 1);
        assert_eq!(cursor.position(), 16 + PREFIX_LEN + ENTRY_LEN);
    }

    #[test]
    fn rejects_unknown_section_version() {
        let bytes = make_flags_bytes(7, &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_feature_flags_section(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("version 7"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_short_prefix() {
        // Section version 1 reads OK, but the count u32 has only 3
        // bytes available.
        let bytes = [FEATURE_FLAGS_SECTION_VERSION, 0, 0, 0];
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_feature_flags_section(&mut cursor) {
            Err(CoreError::TooShort { have, need }) => {
                assert_eq!(have, 3);
                assert_eq!(need, 4);
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn rejects_truncated_entries() {
        let mut bytes = Vec::new();
        bytes.push(FEATURE_FLAGS_SECTION_VERSION);
        bytes.extend_from_slice(&10u32.to_le_bytes());
        bytes.extend_from_slice(&[0x01, 0x00, 0x01]); // 1 entry instead of 10
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_feature_flags_section(&mut cursor) {
            Err(CoreError::TooShort { have, need }) => {
                // Pre-allocation check fires after reading the prefix
                // (5 bytes) but before consuming entries. The 3 entry
                // bytes provided are less than the 30 declared.
                assert_eq!(have, 3);
                assert_eq!(need, 10 * ENTRY_LEN);
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn rejects_zero_flag_id() {
        let bytes = make_flags_bytes(FEATURE_FLAGS_SECTION_VERSION, &[(0x0000, 0x01)]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_feature_flags_section(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("0x0000"), "got: {reason}");
                assert!(reason.contains("reserved"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_required_byte() {
        let bytes = make_flags_bytes(FEATURE_FLAGS_SECTION_VERSION, &[(0x0001, 0x05)]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_feature_flags_section(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("required"), "got: {reason}");
                assert!(reason.contains("0x05"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_duplicate_flag_id() {
        let bytes = make_flags_bytes(
            FEATURE_FLAGS_SECTION_VERSION,
            &[(0x0001, 0x01), (0x0001, 0x00)],
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_feature_flags_section(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("duplicate"), "got: {reason}");
                assert!(reason.contains("0x0001"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_all_standard_flags() {
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
        let mut cursor = ManifestCursor::new(&bytes);
        let flags = parse_feature_flags_section(&mut cursor).expect("all standard parse");
        assert_eq!(flags.len(), standard.len());
        assert_eq!(cursor.position(), PREFIX_LEN + standard.len() * ENTRY_LEN);
        for (entry, (flag_id, required)) in flags.entries.iter().zip(standard.iter()) {
            assert_eq!(entry.flag_id, *flag_id);
            assert_eq!(entry.required, *required != 0);
        }
    }
}
