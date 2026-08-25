//! Metadata reference section (spec §5.3,
//! `bit-level/38-metadata-reference.md`).
//!
//! Carries the BLAKE3 hash of the layer-2 metadata blob plus the
//! locators (or inline bytes) needed to fetch it. The Merkle root
//! (§5.10) commits to `metadata_hash` directly so swapping the
//! metadata blob invalidates the root.
//!
//! ## Section versions
//!
//! - **v1** (original): inline bytes are the uncompressed metadata
//!   blob. Locators (when present) reference an uncompressed
//!   sidecar file.
//! - **v2** (current default for writers): adds a `codec` byte so
//!   the inline bytes (or the sidecar file) can be compressed. The
//!   `metadata_hash` is still BLAKE3 of the **uncompressed** blob;
//!   readers decompress before verifying. v2 is a strict superset of
//!   v1 for readers — old readers reject v2 with `UnsupportedFeature`,
//!   new readers handle both.

use crate::cursor::ManifestCursor;
use crate::error::CoreError;
use crate::locator::{
    parse_locator_entries_with_ceiling, LocatorEntry, DEFAULT_LOCATOR_MAX_URI_BYTES,
};

/// v1 layout (uncompressed inline/external blob).
pub const METADATA_REFERENCE_SECTION_VERSION: u8 = 1;

/// v2 layout (adds `uncompressed_len` + `codec` for compressed blobs).
pub const METADATA_REFERENCE_SECTION_VERSION_2: u8 = 2;

/// Codec id meaning "no compression / stored verbatim". Matches
/// [`crate::codec::CODEC_STORE`].
const CODEC_STORE: u8 = 0x00;

/// Default ceiling on the **compressed** INLINE metadata length (per
/// spec §5.3: "metadata blob ≤ 1 MiB by default"). The uncompressed
/// length is bounded separately and may exceed this when a high-
/// compression codec is in use. Caller can override via
/// [`parse_metadata_reference_with_ceilings`].
///
/// **This gates INLINE metadata only** (bytes carried inside the
/// manifest, parsed before the reader knows anything about the
/// image). EXTERNAL metadata — the `file:metadata.bin` sidecar — is
/// a separate file the opener chose to read and has NO ceiling in
/// the reference load path ([`read_external_metadata`]); use its
/// file size as the bound. Do not apply this constant to sidecars
/// (issue #191: a downstream driver did, rejecting every large-tree
/// image its own format could carry).
pub const DEFAULT_INLINE_METADATA_MAX_BYTES: u32 = 1024 * 1024;

/// Width of the fixed prefix of the v1 section: 1-byte `version` +
/// 32-byte `hash` + 4-byte `locator_count`.
const V1_PREFIX_LEN: usize = 1 + 32 + 4;

/// Width of the fixed prefix of the v2 section: v1 prefix +
/// 4-byte `uncompressed_len` + 1-byte `codec`.
const V2_EXTRA_PREFIX_LEN: usize = 4 + 1;

/// Parsed metadata reference section. The `inline_metadata` field
/// always holds **uncompressed** bytes when present (the parser
/// decompresses v2 blobs transparently). The `codec` field records
/// what was on the wire so callers can re-emit it without
/// re-compression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataReference {
    pub metadata_hash: [u8; 32],
    pub locators: Vec<LocatorEntry>,
    pub inline_metadata: Option<Vec<u8>>,
    /// Codec id used for the inline/external blob on the wire.
    /// `0x00` for v1 (always store). For v2, whatever the writer
    /// chose (typically `0x04` Brotli for source-tree metadata).
    pub codec: u8,
    /// Uncompressed byte length of the metadata blob. Equal to
    /// `inline_metadata.len()` when inline; informative for
    /// external (locator) blobs.
    pub uncompressed_len: u32,
}

impl MetadataReference {
    /// True iff the metadata blob is inlined in this section. Readers
    /// can skip the locator layer when this returns true.
    #[must_use]
    pub fn is_inlined(&self) -> bool {
        self.inline_metadata.is_some()
    }
}

impl Default for MetadataReference {
    fn default() -> Self {
        Self {
            metadata_hash: [0u8; 32],
            locators: Vec::new(),
            inline_metadata: None,
            codec: CODEC_STORE,
            uncompressed_len: 0,
        }
    }
}

/// Parse the metadata reference section from the cursor's current
/// position. Uses the default ceilings (4 KiB per locator URI, 1 MiB
/// inline metadata).
///
/// # Errors
///
/// - [`CoreError::UnsupportedFeature`] if `section_version` is neither
///   1 nor 2.
/// - [`CoreError::Corrupt`] if both `locators` and `inline_metadata`
///   are absent (unreachable metadata), if any structural check fails,
///   or if v2 decompression fails.
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
/// ceilings for the per-locator URI byte length and the **on-wire**
/// (possibly compressed) inline metadata blob length.
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
    match section_version {
        METADATA_REFERENCE_SECTION_VERSION => {
            parse_v1(cursor, max_locator_uri_bytes, max_inline_metadata_bytes)
        }
        METADATA_REFERENCE_SECTION_VERSION_2 => {
            parse_v2(cursor, max_locator_uri_bytes, max_inline_metadata_bytes)
        }
        other => Err(CoreError::UnsupportedFeature {
            feature: format!("metadata_reference section version {other} (supported: 1, 2)"),
        }),
    }
}

fn parse_v1(
    cursor: &mut ManifestCursor<'_>,
    max_locator_uri_bytes: u32,
    max_inline_metadata_bytes: u32,
) -> Result<MetadataReference, CoreError> {
    let metadata_hash = read_hash(cursor)?;
    let locator_count = cursor.read_u32_le()?;
    let locators =
        parse_locator_entries_with_ceiling(cursor, locator_count, max_locator_uri_bytes)?;
    let inline_metadata_len = cursor.read_u32_le()?;
    let inline_metadata = read_inline_blob(
        cursor,
        inline_metadata_len,
        max_inline_metadata_bytes,
        inline_metadata_len,
        CODEC_STORE,
    )?;
    if locators.is_empty() && inline_metadata.is_none() {
        return Err(unreachable_error(V1_PREFIX_LEN));
    }
    Ok(MetadataReference {
        metadata_hash,
        locators,
        inline_metadata,
        codec: CODEC_STORE,
        uncompressed_len: inline_metadata_len,
    })
}

fn parse_v2(
    cursor: &mut ManifestCursor<'_>,
    max_locator_uri_bytes: u32,
    max_inline_metadata_bytes: u32,
) -> Result<MetadataReference, CoreError> {
    let metadata_hash = read_hash(cursor)?;
    let uncompressed_len = cursor.read_u32_le()?;
    let codec = cursor.read_u8()?;
    let locator_count = cursor.read_u32_le()?;
    let locators =
        parse_locator_entries_with_ceiling(cursor, locator_count, max_locator_uri_bytes)?;
    let inline_data_len = cursor.read_u32_le()?;
    let inline_metadata = read_inline_blob(
        cursor,
        inline_data_len,
        max_inline_metadata_bytes,
        uncompressed_len,
        codec,
    )?;
    if locators.is_empty() && inline_metadata.is_none() {
        return Err(unreachable_error(V1_PREFIX_LEN + V2_EXTRA_PREFIX_LEN));
    }
    Ok(MetadataReference {
        metadata_hash,
        locators,
        inline_metadata,
        codec,
        uncompressed_len,
    })
}

fn read_hash(cursor: &mut ManifestCursor<'_>) -> Result<[u8; 32], CoreError> {
    let hash_bytes = cursor.read_n(32)?;
    let mut metadata_hash = [0u8; 32];
    metadata_hash.copy_from_slice(hash_bytes);
    Ok(metadata_hash)
}

/// Read `inline_data_len` bytes from the cursor; if `codec != STORE`,
/// decompress to `uncompressed_len` bytes. Returns `None` if
/// `inline_data_len == 0`. Verifies the decompressed length matches
/// `uncompressed_len`.
fn read_inline_blob(
    cursor: &mut ManifestCursor<'_>,
    inline_data_len: u32,
    max_inline_metadata_bytes: u32,
    uncompressed_len: u32,
    codec: u8,
) -> Result<Option<Vec<u8>>, CoreError> {
    if inline_data_len == 0 {
        return Ok(None);
    }
    if inline_data_len > max_inline_metadata_bytes {
        return Err(CoreError::Corrupt {
            reason: format!(
                "metadata_reference inline_data_len {inline_data_len} exceeds ceiling {max_inline_metadata_bytes}"
            ),
        });
    }
    let wire_len = usize::try_from(inline_data_len).map_err(|_| CoreError::Corrupt {
        reason: format!("metadata_reference inline_data_len {inline_data_len} exceeds usize"),
    })?;
    let wire_bytes = cursor.read_n_owned(wire_len)?;
    if codec == CODEC_STORE {
        return Ok(Some(wire_bytes));
    }
    // Compressed: dispatch to the codec registry.
    let uncompressed =
        crate::codec::decompress(codec, &wire_bytes, uncompressed_len).map_err(|e| {
            CoreError::Corrupt {
                reason: format!("metadata_reference: codec 0x{codec:02X} decompress failed: {e}"),
            }
        })?;
    let got = u32::try_from(uncompressed.len()).unwrap_or(u32::MAX);
    if got != uncompressed_len {
        return Err(CoreError::Corrupt {
            reason: format!(
                "metadata_reference: decompressed length {got} does not match declared uncompressed_len {uncompressed_len}"
            ),
        });
    }
    Ok(Some(uncompressed))
}

fn unreachable_error(prefix_len: usize) -> CoreError {
    CoreError::Corrupt {
        reason: format!(
            "metadata_reference is unreachable: locator_count=0 and inline_data_len=0 (need at least one source for the {prefix_len}-byte metadata blob)"
        ),
    }
}

/// Load the metadata blob bytes for an image whose reference section
/// parsed to external locators: follow the first `file:` locator
/// (resolved relative to the image file's directory), then decompress
/// per the reference's codec field (codec 0 = STORE returns the raw
/// bytes). Images with INLINE metadata don't call this — their bytes
/// already live in [`MetadataReference::inline_metadata`].
///
/// This is the one true load path for external metadata. It applies
/// NO size ceiling: the sidecar is a file the caller chose to open,
/// so its on-disk size is the bound — unlike INLINE metadata, where
/// [`DEFAULT_INLINE_METADATA_MAX_BYTES`] protects the unbounded
/// manifest read. Verified at 150,000 inodes / 616 MiB sidecar
/// (issue #191).
///
/// # Errors
///
/// - [`CoreError::Corrupt`] if the reference carries no locators, or
///   if the locator is not a `file:` URI.
/// - [`CoreError::Io`]-shaped [`CoreError::Corrupt`] if the sidecar
///   cannot be read, or v2 decompression fails.
pub fn read_external_metadata(
    reference: &MetadataReference,
    image_path: &std::path::Path,
) -> Result<Vec<u8>, CoreError> {
    let entry = reference
        .locators
        .first()
        .ok_or_else(|| CoreError::Corrupt {
            reason: "metadata_reference has neither inline data nor locators".into(),
        })?;
    if !entry.uri.starts_with("file:") {
        return Err(CoreError::Corrupt {
            reason: format!(
                "metadata_reference locator {} is not a file: URI; \
                 external metadata requires a local sidecar",
                entry.uri
            ),
        });
    }
    let name = entry.uri.strip_prefix("file:").unwrap_or(&entry.uri);
    let sidecar_path = image_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(name);
    let wire = std::fs::read(&sidecar_path).map_err(|e| CoreError::Corrupt {
        reason: format!("read external metadata {}: {e}", sidecar_path.display()),
    })?;
    if reference.codec == 0 {
        Ok(wire)
    } else {
        crate::codec::decompress(reference.codec, &wire, reference.uncompressed_len)
    }
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

    /// Build a v2 `metadata_reference` section with `codec` + the given
    /// on-wire bytes.
    fn make_v2_bytes(
        metadata_hash: [u8; 32],
        uncompressed_len: u32,
        codec: u8,
        locator_uris: &[&str],
        inline_data: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(METADATA_REFERENCE_SECTION_VERSION_2);
        bytes.extend_from_slice(&metadata_hash);
        bytes.extend_from_slice(&uncompressed_len.to_le_bytes());
        bytes.push(codec);
        let locator_count = u32::try_from(locator_uris.len()).expect("count fits u32");
        bytes.extend_from_slice(&locator_count.to_le_bytes());
        for uri in locator_uris {
            bytes.extend(make_locator_bytes(uri));
        }
        let inline_len =
            inline_data.map_or(0u32, |b| u32::try_from(b.len()).expect("len fits u32"));
        bytes.extend_from_slice(&inline_len.to_le_bytes());
        if let Some(blob) = inline_data {
            bytes.extend(blob);
        }
        bytes
    }

    #[test]
    fn v2_store_codec_round_trips() {
        let blob = b"hello metadata blob world";
        let hash = crate::merkle::hash_section(blob);
        let bytes = make_v2_bytes(hash, blob.len() as u32, 0x00, &[], Some(blob));
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_metadata_reference(&mut cursor).expect("v2 parses");
        assert_eq!(parsed.metadata_hash, hash);
        assert_eq!(parsed.codec, 0x00);
        assert_eq!(parsed.uncompressed_len, u32::try_from(blob.len()).unwrap());
        assert_eq!(parsed.inline_metadata.as_deref(), Some(blob.as_slice()));
    }

    #[test]
    fn v2_brotli_codec_decompresses_inline() {
        let blob = b"the quick brown fox jumps over the lazy dog".repeat(50);
        let hash = crate::merkle::hash_section(&blob);
        // Compress with the registry's brotli codec (CODEC_BROTLI = 0x04).
        let compressed = crate::codec::compress(0x04, &blob).expect("brotli compress");
        assert!(
            compressed.len() < blob.len(),
            "brotli should beat store on repetitive input"
        );
        let bytes = make_v2_bytes(
            hash,
            u32::try_from(blob.len()).unwrap(),
            0x04,
            &[],
            Some(&compressed),
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_metadata_reference(&mut cursor).expect("v2 brotli parses");
        assert_eq!(parsed.codec, 0x04);
        assert_eq!(parsed.uncompressed_len, u32::try_from(blob.len()).unwrap());
        assert_eq!(parsed.inline_metadata.as_deref(), Some(blob.as_slice()));
    }

    #[test]
    fn v2_rejects_uncompressed_length_mismatch() {
        let blob = b"hello";
        let compressed = crate::codec::compress(0x04, blob).expect("brotli");
        // Lie about uncompressed_len: claim 999 instead of 5.
        let hash = crate::merkle::hash_section(blob);
        let bytes = make_v2_bytes(hash, 999, 0x04, &[], Some(&compressed));
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_metadata_reference(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                // Either the codec rejects the wrong expected_len, or
                // our post-decode length check fires. Both are
                // acceptable rejections of the inconsistent input.
                assert!(
                    reason.contains("length") || reason.contains("decompress"),
                    "got: {reason}"
                );
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn unknown_section_version_above_2_rejected() {
        let bytes = make_metadata_reference_bytes(99, [0u8; 32], &[], Some(b"inline blob"));
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_metadata_reference(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("version 99"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }
}
