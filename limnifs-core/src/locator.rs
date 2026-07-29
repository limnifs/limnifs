//! Locator entry (spec §12, `bit-level/37-locator-entry.md`).
//!
//! A length-prefixed URI in the form `scheme ":" scheme_specific_part`.
//! Locator entries appear inside larger sections (metadata reference
//! §5.3, slab index §5.4). Readers race alternatives per §I9 when
//! multiple entries exist for one blob.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::cursor::ManifestCursor;
use crate::error::CoreError;

/// Width of the u32 LE length prefix on every locator entry.
pub const LOCATOR_LENGTH_PREFIX_LEN: usize = 4;

/// Default per-locator URI byte ceiling. The manifest's parameters
/// section may override.
pub const DEFAULT_LOCATOR_MAX_URI_BYTES: u32 = 4 * 1024;

/// Smallest meaningful URI: one-letter scheme plus `://`. Lengths
/// below this are `Corrupt`.
pub const MIN_LOCATOR_URI_BYTES: u32 = 4;

/// A parsed locator entry. The URI is owned so the entry outlives the
/// cursor's borrow.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct LocatorEntry {
    pub uri: String,
}

impl LocatorEntry {
    /// Extract the scheme (substring before the first `:`). Returns
    /// `None` if there is no colon — but parsers reject such inputs
    /// up front, so callers can treat this as infallible for any
    /// locator that came from [`parse_locator_entry`].
    #[must_use]
    pub fn scheme(&self) -> Option<&str> {
        self.uri.split_once(':').map(|(scheme, _)| scheme)
    }

    /// Extract the scheme-specific part (everything after the first
    /// `:`). Returns `None` if there is no colon.
    #[must_use]
    pub fn scheme_specific_part(&self) -> Option<&str> {
        self.uri.split_once(':').map(|(_, rest)| rest)
    }
}

/// Parse a single locator entry from the cursor's current position.
///
/// Reads the u32 LE length prefix, then the URI bytes. Performs the
/// structural checks: minimum length, maximum length (default 4 KiB),
/// UTF-8 validity, presence of a `:` separator, and RFC 3986 scheme
/// grammar.
///
/// Does NOT check whether the scheme is one the reader implements —
/// that policy belongs to the locator-racing layer (§I9), not the
/// parser.
///
/// # Errors
///
/// - [`CoreError::TooShort`] if the cursor has fewer than
///   `4 + length` bytes.
/// - [`CoreError::Corrupt`] if `length < 4`, if `length` exceeds the
///   configured ceiling, if the URI bytes are not valid UTF-8, if no
///   `:` separator is present, or if the scheme does not match RFC
///   3986 grammar.
pub fn parse_locator_entry(cursor: &mut ManifestCursor<'_>) -> Result<LocatorEntry, CoreError> {
    parse_locator_entry_with_ceiling(cursor, DEFAULT_LOCATOR_MAX_URI_BYTES)
}

/// Same as [`parse_locator_entry`] but lets the caller supply a
/// `max_uri_bytes` overriding the 4 KiB default.
///
/// # Errors
///
/// Inherits all errors from [`parse_locator_entry`].
pub fn parse_locator_entry_with_ceiling(
    cursor: &mut ManifestCursor<'_>,
    max_uri_bytes: u32,
) -> Result<LocatorEntry, CoreError> {
    let raw_length = cursor.read_u32_le()?;
    if raw_length < MIN_LOCATOR_URI_BYTES {
        return Err(CoreError::Corrupt {
            reason: format!("locator length {raw_length} is below minimum {MIN_LOCATOR_URI_BYTES}"),
        });
    }
    if raw_length > max_uri_bytes {
        return Err(CoreError::Corrupt {
            reason: format!("locator length {raw_length} exceeds ceiling {max_uri_bytes}"),
        });
    }
    let length = usize::try_from(raw_length).map_err(|_| CoreError::Corrupt {
        reason: format!("locator length {raw_length} exceeds usize"),
    })?;
    let uri_bytes = cursor.read_n(length)?;
    let uri = std::str::from_utf8(uri_bytes).map_err(|_| CoreError::Corrupt {
        reason: format!("locator URI is not valid UTF-8 ({length} bytes)"),
    })?;
    let (scheme, rest) = uri.split_once(':').ok_or_else(|| CoreError::Corrupt {
        reason: format!("locator URI {uri:?} missing scheme separator ':'"),
    })?;
    if scheme.is_empty() {
        return Err(CoreError::Corrupt {
            reason: format!("locator URI {uri:?} has empty scheme"),
        });
    }
    if !is_valid_scheme(scheme) {
        return Err(CoreError::Corrupt {
            reason: format!(
                "locator URI {uri:?} has scheme {scheme:?} that does not match RFC 3986 grammar"
            ),
        });
    }
    if rest.is_empty() {
        return Err(CoreError::Corrupt {
            reason: format!("locator URI {uri:?} has empty scheme-specific part"),
        });
    }
    Ok(LocatorEntry {
        uri: uri.to_owned(),
    })
}

/// RFC 3986 section 3.1: `scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`.
fn is_valid_scheme(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    let first = chars.next();
    if !first.is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_locator_bytes(uri: &str) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(LOCATOR_LENGTH_PREFIX_LEN + uri.len());
        let length = u32::try_from(uri.len()).expect("test URI fits u32");
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(uri.as_bytes());
        bytes
    }

    #[test]
    fn parses_file_uri() {
        let uri = "file:///var/lib/limnifs/slab-7.bin";
        let bytes = make_locator_bytes(uri);
        let mut cursor = ManifestCursor::new(&bytes);
        let entry = parse_locator_entry(&mut cursor).expect("file URI parses");
        assert_eq!(entry.uri, uri);
        assert_eq!(entry.scheme(), Some("file"));
        assert_eq!(
            entry.scheme_specific_part(),
            Some("///var/lib/limnifs/slab-7.bin")
        );
        assert_eq!(cursor.position(), bytes.len());
    }

    #[test]
    fn parses_https_uri_with_query() {
        let uri = "https://cdn.example.com/slabs/7.bin?range=0-4095";
        let bytes = make_locator_bytes(uri);
        let mut cursor = ManifestCursor::new(&bytes);
        let entry = parse_locator_entry(&mut cursor).expect("https URI parses");
        assert_eq!(entry.scheme(), Some("https"));
    }

    #[test]
    fn parses_s3_uri() {
        let uri = "s3://my-bucket/slabs/7.bin?region=us-east-1";
        let bytes = make_locator_bytes(uri);
        let mut cursor = ManifestCursor::new(&bytes);
        let entry = parse_locator_entry(&mut cursor).expect("s3 URI parses");
        assert_eq!(entry.scheme(), Some("s3"));
    }

    #[test]
    fn parses_ipfs_uri() {
        let uri = "ipfs://bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";
        let bytes = make_locator_bytes(uri);
        let mut cursor = ManifestCursor::new(&bytes);
        let entry = parse_locator_entry(&mut cursor).expect("ipfs URI parses");
        assert_eq!(entry.scheme(), Some("ipfs"));
    }

    #[test]
    fn parses_limni_p2p_uri_with_plus_and_dash() {
        // Scheme `limni-p2p` exercises the `-` grammar; an extra
        // `+` in the scheme position is also valid per RFC 3986.
        let uri = "limni-p2p://12D3KooWabc/some-hash";
        let bytes = make_locator_bytes(uri);
        let mut cursor = ManifestCursor::new(&bytes);
        let entry = parse_locator_entry(&mut cursor).expect("limni-p2p URI parses");
        assert_eq!(entry.scheme(), Some("limni-p2p"));
    }

    #[test]
    fn rejects_length_below_minimum() {
        let bytes = 3u32.to_le_bytes();
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_locator_entry(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("minimum"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_length_above_default_ceiling() {
        let bytes = (DEFAULT_LOCATOR_MAX_URI_BYTES + 1).to_le_bytes();
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_locator_entry(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("ceiling"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn custom_ceiling_accepts_longer_uri() {
        let long_uri = format!("file:///{}", "a".repeat(8192));
        let bytes = make_locator_bytes(&long_uri);
        let mut cursor = ManifestCursor::new(&bytes);
        let entry = parse_locator_entry_with_ceiling(&mut cursor, 16 * 1024)
            .expect("custom ceiling accepts");
        assert_eq!(entry.uri, long_uri);
    }

    #[test]
    fn rejects_non_utf8_uri() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(b"ab\xff\xfe:"); // invalid UTF-8 + colon
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_locator_entry(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("UTF-8"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_missing_colon() {
        let bytes = make_locator_bytes("abcde");
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_locator_entry(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("separator"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_scheme_starting_with_digit() {
        let bytes = make_locator_bytes("1abc://example.com/");
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_locator_entry(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("RFC 3986"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_scheme_with_invalid_character() {
        let bytes = make_locator_bytes("ab c://example.com/");
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_locator_entry(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("RFC 3986"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_scheme_specific_part() {
        let bytes = make_locator_bytes("file:");
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_locator_entry(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("empty scheme-specific"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_truncated_uri_body() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&100u32.to_le_bytes()); // claim 100 bytes
        bytes.extend_from_slice(b"file://short"); // only 11 bytes
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_locator_entry(&mut cursor) {
            Err(CoreError::TooShort { .. }) => {}
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn rejects_truncated_length_prefix() {
        let bytes = [0u8; 3];
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_locator_entry(&mut cursor) {
            Err(CoreError::TooShort { .. }) => {}
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn parses_two_consecutive_entries() {
        let mut bytes = Vec::new();
        bytes.extend(make_locator_bytes("file:///a.bin"));
        bytes.extend(make_locator_bytes("https://cdn/b.bin"));
        let mut cursor = ManifestCursor::new(&bytes);
        let first = parse_locator_entry(&mut cursor).expect("first parses");
        let second = parse_locator_entry(&mut cursor).expect("second parses");
        assert_eq!(first.scheme(), Some("file"));
        assert_eq!(second.scheme(), Some("https"));
        assert_eq!(cursor.position(), bytes.len());
    }
}
