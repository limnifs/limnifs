//! Slab index section (spec §5.4, `bit-level/39-slab-index.md`).
//!
//! Per-slab table of contents: one entry per slab referenced by this
//! image. Each entry carries the `SlabId` and one or more locator
//! entries that can fetch the slab bytes.

use crate::cursor::ManifestCursor;
use crate::error::CoreError;
use crate::locator::{
    parse_locator_entries_with_ceiling, LocatorEntry, DEFAULT_LOCATOR_MAX_URI_BYTES,
};
use limnifs_format::SlabId;

/// Current layout version of this section.
pub const SLAB_INDEX_SECTION_VERSION: u8 = 1;

/// Width of the fixed prefix of the section (version byte + u32 LE count).
const PREFIX_LEN: usize = 1 + 4;

/// Width of the `SlabId` + `locator_count` prefix of each entry.
const ENTRY_FIXED_LEN: usize = 40 + 4;

/// Parsed slab index entry. One per slab referenced by this image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlabIndexEntry {
    pub slab_id: SlabId,
    pub locators: Vec<LocatorEntry>,
}

/// Parsed slab index section.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct SlabIndex {
    pub entries: Vec<SlabIndexEntry>,
}

impl SlabIndex {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Look up the entry for `slab_id`, if present.
    #[must_use]
    pub fn find(&self, slab_id: &SlabId) -> Option<&SlabIndexEntry> {
        self.entries.iter().find(|entry| &entry.slab_id == slab_id)
    }
}

/// Parse the slab index section from the cursor's current position.
/// Uses the default 4 KiB per-locator URI ceiling.
///
/// # Errors
///
/// - [`CoreError::UnsupportedFeature`] if `section_version` is not 1.
/// - [`CoreError::Corrupt`] if `entry_count` is zero (degenerate image;
///   empty-image cross-check runs at the slab-walker layer), if any
///   `locator_count` is zero (unreachable slab), or if a `slab_id`
///   appears more than once.
/// - Inherits errors from [`crate::locator::parse_locator_entries`].
pub fn parse_slab_index(cursor: &mut ManifestCursor<'_>) -> Result<SlabIndex, CoreError> {
    parse_slab_index_with_ceiling(cursor, DEFAULT_LOCATOR_MAX_URI_BYTES)
}

/// Same as [`parse_slab_index`] but with a caller-supplied per-locator
/// URI byte ceiling.
///
/// # Errors
///
/// Inherits all errors from [`parse_slab_index`].
pub fn parse_slab_index_with_ceiling(
    cursor: &mut ManifestCursor<'_>,
    max_locator_uri_bytes: u32,
) -> Result<SlabIndex, CoreError> {
    let section_version = cursor.read_u8()?;
    if section_version != SLAB_INDEX_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "slab_index section version {section_version} (supported: {SLAB_INDEX_SECTION_VERSION})"
            ),
        });
    }
    let raw_count = cursor.read_u32_le()?;
    let entry_count = usize::try_from(raw_count).map_err(|_| CoreError::Corrupt {
        reason: format!("slab_index entry count {raw_count} exceeds usize"),
    })?;
    // DoS check: each entry needs at least ENTRY_FIXED_LEN bytes
    // (40-byte slab_id + 4-byte locator_count, before any locator
    // bodies). Reject before allocating.
    let min_section_size =
        entry_count
            .checked_mul(ENTRY_FIXED_LEN)
            .ok_or_else(|| CoreError::Corrupt {
                reason: format!("slab_index entry count {entry_count} overflows usize"),
            })?;
    if cursor.remaining_len() < min_section_size {
        return Err(CoreError::TooShort {
            have: cursor.remaining_len(),
            need: min_section_size,
        });
    }
    let mut entries = Vec::with_capacity(entry_count);
    for index in 0..entry_count {
        let ordinal = cursor.read_u64_le()?;
        let hash_bytes = cursor.read_n(32)?;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(hash_bytes);
        let slab_id = SlabId::new(ordinal, hash);
        if entries
            .iter()
            .any(|existing: &SlabIndexEntry| existing.slab_id == slab_id)
        {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "slab_index entry {index}: duplicate slab_id (ordinal {}, hash {})",
                    slab_id.ordinal,
                    hex_lower(&slab_id.hash)
                ),
            });
        }
        let locator_count = cursor.read_u32_le()?;
        if locator_count == 0 {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "slab_index entry {index}: slab_id (ordinal {}) declares zero locators (unreachable)",
                    slab_id.ordinal
                ),
            });
        }
        let locators =
            parse_locator_entries_with_ceiling(cursor, locator_count, max_locator_uri_bytes)?;
        entries.push(SlabIndexEntry { slab_id, locators });
    }
    let _ = PREFIX_LEN; // documented constant; emit nothing here
    Ok(SlabIndex { entries })
}

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(s, "{byte:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_locator_bytes(uri: &str) -> Vec<u8> {
        let length = u32::try_from(uri.len()).expect("test URI fits u32");
        let mut bytes = Vec::with_capacity(4 + uri.len());
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(uri.as_bytes());
        bytes
    }

    fn make_entry_bytes(slab_id: SlabId, locator_uris: &[&str]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let slab_bytes = slab_id.to_bytes();
        bytes.extend_from_slice(&slab_bytes);
        let count = u32::try_from(locator_uris.len()).expect("count fits u32");
        bytes.extend_from_slice(&count.to_le_bytes());
        for uri in locator_uris {
            bytes.extend(make_locator_bytes(uri));
        }
        bytes
    }

    fn make_slab_index_bytes(version: u8, entries: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(version);
        let count = u32::try_from(entries.len()).expect("count fits u32");
        bytes.extend_from_slice(&count.to_le_bytes());
        for entry in entries {
            bytes.extend_from_slice(entry);
        }
        bytes
    }

    fn sample_slab_id(ordinal: u64) -> SlabId {
        let mut hash = [0u8; 32];
        hash[0] = u8::try_from(ordinal).expect("test ordinal fits u8");
        SlabId::new(ordinal, hash)
    }

    #[test]
    fn parses_single_slab_single_locator() {
        let slab = sample_slab_id(0);
        let entry = make_entry_bytes(slab, &["file:///var/lib/limnifs/slab-0.bin"]);
        let bytes = make_slab_index_bytes(SLAB_INDEX_SECTION_VERSION, &[entry]);
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_slab_index(&mut cursor).expect("single slab parses");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed.entries[0].slab_id, slab);
        assert_eq!(parsed.entries[0].locators.len(), 1);
        assert_eq!(cursor.position(), bytes.len());
    }

    #[test]
    fn parses_two_slabs_mirrored() {
        let slab_a = sample_slab_id(0);
        let slab_b = sample_slab_id(1);
        let entry_a = make_entry_bytes(slab_a, &["file:///a.bin", "https://cdn/a.bin"]);
        let entry_b = make_entry_bytes(slab_b, &["file:///b.bin", "https://cdn/b.bin"]);
        let bytes = make_slab_index_bytes(SLAB_INDEX_SECTION_VERSION, &[entry_a, entry_b]);
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_slab_index(&mut cursor).expect("two slabs parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.entries[0].locators.len(), 2);
        assert_eq!(parsed.entries[1].locators.len(), 2);
        assert!(parsed.find(&slab_a).is_some());
        assert!(parsed.find(&slab_b).is_some());
    }

    #[test]
    fn rejects_unknown_section_version() {
        let bytes = make_slab_index_bytes(7, &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_slab_index(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("version 7"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_entry_count_overrunning_buffer() {
        // Declare 10 entries but provide 0 entry bytes.
        let mut bytes = Vec::new();
        bytes.push(SLAB_INDEX_SECTION_VERSION);
        bytes.extend_from_slice(&10u32.to_le_bytes());
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_slab_index(&mut cursor) {
            Err(CoreError::TooShort { have, need }) => {
                assert_eq!(have, 0);
                assert_eq!(need, 10 * ENTRY_FIXED_LEN);
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn rejects_duplicate_slab_id() {
        let slab = sample_slab_id(5);
        let entry = make_entry_bytes(slab, &["file:///x.bin"]);
        let bytes = make_slab_index_bytes(SLAB_INDEX_SECTION_VERSION, &[entry.clone(), entry]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_slab_index(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("duplicate"), "got: {reason}");
                assert!(reason.contains("ordinal 5"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_slab_with_zero_locators() {
        let slab = sample_slab_id(0);
        let entry = make_entry_bytes(slab, &[]);
        let bytes = make_slab_index_bytes(SLAB_INDEX_SECTION_VERSION, &[entry]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_slab_index(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("zero locators"), "got: {reason}");
                assert!(reason.contains("unreachable"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_truncated_prefix() {
        // Valid version + partial entry_count. Cursor returns
        // TooShort when it reaches the missing bytes.
        let mut bytes = vec![SLAB_INDEX_SECTION_VERSION];
        bytes.extend_from_slice(&[0u8; 2]); // 2 bytes of count, need 4
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_slab_index(&mut cursor) {
            Err(CoreError::TooShort { .. }) => {}
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn helper_hex_lower_is_lowercase() {
        let s = hex_lower(&[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(s, "deadbeef");
    }
}
