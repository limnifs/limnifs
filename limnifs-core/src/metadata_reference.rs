//! Metadata reference section (spec §5.3,
//! `bit-level/38-metadata-reference.md`).
//!
//! Carries the BLAKE3 hash of the layer-2 metadata blob plus the
//! locators (or inline bytes) needed to fetch it. The Merkle root
//! (§5.10) commits to `metadata_hash` directly so swapping the
//! metadata blob invalidates the root.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::cursor::ManifestCursor;
use crate::error::CoreError;
use crate::locator::{
    parse_locator_entries_with_ceiling, LocatorEntry, DEFAULT_LOCATOR_MAX_URI_BYTES,
};

/// Current layout version of this section.
pub const METADATA_REFERENCE_SECTION_VERSION: u8 = 1;

/// Default ceiling on the inline metadata blob length (per spec §5.3:
/// "metadata blob ≤ 1 MiB by default"). Caller can override via
/// [`parse_metadata_reference_with_ceilings`].
pub const DEFAULT_INLINE_METADATA_MAX_BYTES: u32 = 1024 * 1024;

/// Width of the fixed prefix of this section: 1-byte `version` +
/// 32-byte `hash` + 4-byte `locator_count`.
const PREFIX_LEN: usize = 1 + 32 + 4;

/// Parsed metadata reference section.
///
/// `metadata_hash` is the BLAKE3 of the layer-2 metadata blob; readers
/// verify the fetched (or inlined) bytes hash to exactly this value.
/// Either `locators` is non-empty (external metadata) or
/// `inline_metadata` is `Some` (inlined metadata); at least one must
/// hold, enforced by the parser.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataReference {
    pub metadata_hash: [u8; 32],
    pub locators: Vec<LocatorEntry>,
    pub inline_metadata: Option<Vec<u8>>,
}

impl MetadataReference {
    /// True iff the metadata blob is inlined in this section. Readers
    /// can skip the locator layer when this returns true.
    #[must_use]
    pub fn is_inlined(&self) -> bool {
        self.inline_metadata.is_some()
    }
}

/// Parse the metadata reference section from the cursor's current
/// position. Uses the default ceilings (4 KiB per locator URI, 1 MiB
/// inline metadata).
///
/// # Errors
///
/// - [`CoreError::UnsupportedFeature`] if `section_version` is not 1.
/// - [`CoreError::Corrupt`] if both `locators` and `inline_metadata`
///   are absent (unreachable metadata), or if any structural check
///   fails.
/// - Inherits errors from [`crate::locator::parse_locator_entries`].
pub fn parse_metadata_reference(
    cursor: &mut ManifestCursor<'_>,
) -> Result<MetadataReference, CoreError> {
    parse_metadata_reference_with_ceilings(
        cursor,
        DEFAULT_LOCATOR_MAX_URI_BYTES,
        DEFAULT_INLINE_METADATA_MAX_BYTES,
    )
}

/// Same as [`parse_metadata_reference`] but with caller-supplied
/// ceilings for the per-locator URI byte length and the inline
/// metadata blob length.
///
/// # Errors
///
/// Inherits all errors from [`parse_metadata_reference`].
pub fn parse_metadata_reference_with_ceilings(
    cursor: &mut ManifestCursor<'_>,
    max_locator_uri_bytes: u32,
    max_inline_metadata_bytes: u32,
) -> Result<MetadataReference, CoreError> {
    let section_version = cursor.read_u8()?;
    if section_version != METADATA_REFERENCE_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "metadata_reference section version {section_version} (supported: {METADATA_REFERENCE_SECTION_VERSION})"
            ),
        });
    }
    let hash_bytes = cursor.read_n(32)?;
    let mut metadata_hash = [0u8; 32];
    metadata_hash.copy_from_slice(hash_bytes);
    let locator_count = cursor.read_u32_le()?;
    let locators =
        parse_locator_entries_with_ceiling(cursor, locator_count, max_locator_uri_bytes)?;
    let inline_metadata_len = cursor.read_u32_le()?;
    let inline_metadata = if inline_metadata_len == 0 {
        None
    } else if inline_metadata_len > max_inline_metadata_bytes {
        return Err(CoreError::Corrupt {
            reason: format!(
                "metadata_reference inline_metadata_len {inline_metadata_len} exceeds ceiling {max_inline_metadata_bytes}"
            ),
        });
    } else {
        let length = usize::try_from(inline_metadata_len).map_err(|_| CoreError::Corrupt {
            reason: format!(
                "metadata_reference inline_metadata_len {inline_metadata_len} exceeds usize"
            ),
        })?;
        Some(cursor.read_n_owned(length)?)
    };
    if locators.is_empty() && inline_metadata.is_none() {
        return Err(CoreError::Corrupt {
            reason: format!(
                "metadata_reference is unreachable: locator_count=0 and inline_metadata_len=0 (need at least one source for the {PREFIX_LEN}-byte metadata blob)"
            ),
        });
    }
    Ok(MetadataReference {
        metadata_hash,
        locators,
        inline_metadata,
    })
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

    fn make_metadata_reference_bytes(
        version: u8,
        metadata_hash: [u8; 32],
        locator_uris: &[&str],
        inline_metadata: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(version);
        bytes.extend_from_slice(&metadata_hash);
        let locator_count = u32::try_from(locator_uris.len()).expect("count fits u32");
        bytes.extend_from_slice(&locator_count.to_le_bytes());
        for uri in locator_uris {
            bytes.extend(make_locator_bytes(uri));
        }
        let inline_len =
            inline_metadata.map_or(0u32, |b| u32::try_from(b.len()).expect("len fits u32"));
        bytes.extend_from_slice(&inline_len.to_le_bytes());
        if let Some(blob) = inline_metadata {
            bytes.extend_from_slice(blob);
        }
        bytes
    }

    fn sample_hash() -> [u8; 32] {
        [0xAA; 32]
    }

    #[test]
    fn parses_external_metadata_single_locator() {
        let bytes = make_metadata_reference_bytes(
            METADATA_REFERENCE_SECTION_VERSION,
            sample_hash(),
            &["file:///var/lib/limnifs/metadata.bin"],
            None,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_metadata_reference(&mut cursor).expect("external parses");
        assert_eq!(parsed.metadata_hash, sample_hash());
        assert_eq!(parsed.locators.len(), 1);
        assert_eq!(parsed.locators[0].scheme(), Some("file"));
        assert!(parsed.inline_metadata.is_none());
        assert!(!parsed.is_inlined());
        assert_eq!(cursor.position(), bytes.len());
    }

    #[test]
    fn parses_inlined_metadata() {
        let blob = vec![0xBB; 1024];
        let bytes = make_metadata_reference_bytes(
            METADATA_REFERENCE_SECTION_VERSION,
            sample_hash(),
            &[],
            Some(&blob),
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_metadata_reference(&mut cursor).expect("inlined parses");
        assert_eq!(parsed.locators.len(), 0);
        assert_eq!(parsed.inline_metadata.as_deref(), Some(&blob[..]));
        assert!(parsed.is_inlined());
    }

    #[test]
    fn parses_mirrored_with_inline_fallback() {
        let blob = vec![0xCC; 4096];
        let bytes = make_metadata_reference_bytes(
            METADATA_REFERENCE_SECTION_VERSION,
            sample_hash(),
            &["https://cdn/x.bin", "s3://bucket/x.bin"],
            Some(&blob),
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_metadata_reference(&mut cursor).expect("mirrored parses");
        assert_eq!(parsed.locators.len(), 2);
        assert_eq!(parsed.locators[0].scheme(), Some("https"));
        assert_eq!(parsed.locators[1].scheme(), Some("s3"));
        assert!(parsed.inline_metadata.is_some());
    }

    #[test]
    fn rejects_unknown_section_version() {
        let bytes = make_metadata_reference_bytes(7, sample_hash(), &["file:///x"], None);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_metadata_reference(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("version 7"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unreachable_metadata() {
        let bytes = make_metadata_reference_bytes(
            METADATA_REFERENCE_SECTION_VERSION,
            sample_hash(),
            &[],
            None,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_metadata_reference(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("unreachable"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_inline_above_default_ceiling() {
        let oversized = vec![0xDD; (DEFAULT_INLINE_METADATA_MAX_BYTES as usize) + 1];
        let bytes = make_metadata_reference_bytes(
            METADATA_REFERENCE_SECTION_VERSION,
            sample_hash(),
            &[],
            Some(&oversized),
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_metadata_reference(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("ceiling"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn custom_ceiling_accepts_oversized_inline() {
        let oversized = vec![0xEE; (DEFAULT_INLINE_METADATA_MAX_BYTES as usize) + 1024];
        let bytes = make_metadata_reference_bytes(
            METADATA_REFERENCE_SECTION_VERSION,
            sample_hash(),
            &[],
            Some(&oversized),
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_metadata_reference_with_ceilings(
            &mut cursor,
            DEFAULT_LOCATOR_MAX_URI_BYTES,
            2 * DEFAULT_INLINE_METADATA_MAX_BYTES,
        )
        .expect("custom ceiling accepts");
        assert_eq!(parsed.inline_metadata.as_deref(), Some(&oversized[..]));
    }

    #[test]
    fn rejects_truncated_prefix() {
        // Valid version + partial hash. Cursor returns TooShort when
        // it reaches the missing bytes of the 32-byte hash.
        let mut bytes = vec![METADATA_REFERENCE_SECTION_VERSION];
        bytes.extend_from_slice(&[0u8; 30]); // 30 bytes of hash, need 32
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_metadata_reference(&mut cursor) {
            Err(CoreError::TooShort { .. }) => {}
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_locator_entry_inherited() {
        // Locator with no colon; the inner error should propagate.
        let mut bytes = Vec::new();
        bytes.push(METADATA_REFERENCE_SECTION_VERSION);
        bytes.extend_from_slice(&sample_hash());
        bytes.extend_from_slice(&1u32.to_le_bytes()); // locator_count = 1
        bytes.extend_from_slice(&5u32.to_le_bytes()); // length = 5
        bytes.extend_from_slice(b"abcde"); // no colon
        bytes.extend_from_slice(&0u32.to_le_bytes()); // inline = 0
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_metadata_reference(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("separator"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }
}
