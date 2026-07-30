//! Slab compaction — removes unreferenced drops from a slab without
//! re-reading file contents. Preserves codecs, drop identities, and
//! metadata; only the slab and manifest's slab index / history change.
//!
//! ## Algorithm
//!
//! 1. Parse the source manifest's prefix (header, flags, metadata
//!    reference — these are copied verbatim).
//! 2. Walk the metadata blob to find all referenced `DropId`s.
//! 3. Load the source slab, extract referenced drops with their
//!    compressed bytes and codec (no decompression needed).
//! 4. Build a new slab containing only the referenced drops.
//! 5. Re-encode the slab index + history.
//! 6. Recompute the Merkle root.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::collections::HashSet;

use limnifs_core::{
    compute_merkle_root, hash_empty_section, hash_section, parse_drop_record,
    parse_feature_flags_section, parse_manifest_header, parse_metadata_blob,
    parse_metadata_reference, parse_slab_header, parse_slab_index, ContentHandle, CoreError,
    ManifestCursor, SectionHashes, HISTORY_SECTION_VERSION, SLAB_INDEX_SECTION_VERSION,
};
use limnifs_format::{ManifestRoot, SlabId};

/// Result of compacting an image.
#[derive(Clone, Debug)]
pub struct CompactionResult {
    pub manifest_bytes: Vec<u8>,
    pub merkle_root: ManifestRoot,
    pub slab_bytes: Option<Vec<u8>>,
    pub original_drop_count: usize,
    pub compacted_drop_count: usize,
    pub reclaimed_drops: usize,
}

/// Error during compaction.
#[derive(Debug)]
pub enum CompactionError {
    Core(CoreError),
    Io(std::io::Error),
}

impl std::fmt::Display for CompactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "I/O: {e}"),
        }
    }
}

impl std::error::Error for CompactionError {}

impl From<CoreError> for CompactionError {
    fn from(e: CoreError) -> Self {
        Self::Core(e)
    }
}

impl From<std::io::Error> for CompactionError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// One drop extracted from the source slab, ready for re-packing.
struct ExtractedDrop {
    id: [u8; 32],
    compressed: Vec<u8>,
    codec: u8,
    plaintext_len: u32,
}

/// Compact an image by removing unreferenced drops from its slab.
/// The manifest's metadata, header, and flags are preserved; only the
/// slab and the slab index / history sections are updated.
///
/// # Errors
///
/// Returns [`CompactionError`] if the image or slab cannot be parsed.
pub fn compact_image(
    manifest_bytes: &[u8],
    slab_bytes: &[u8],
) -> Result<CompactionResult, CompactionError> {
    // 1. Parse the manifest prefix to find referenced drops.
    let mut cursor = ManifestCursor::new(manifest_bytes);
    let _header = parse_manifest_header(&mut cursor)?;

    // Capture the raw prefix bytes (header + flags + metadata ref)
    // for verbatim re-encoding.
    let prefix_end = {
        let flags_start = cursor.position();
        let _ = parse_feature_flags_section(&mut cursor)?;
        let flags_end = cursor.position();
        let meta_ref_start = cursor.position();
        let meta_ref = parse_metadata_reference(&mut cursor)?;
        let _meta_ref_end = cursor.position();
        let _ = (flags_start, flags_end, meta_ref_start);

        // Walk inodes to find referenced drops.
        let referenced = find_referenced_drops(&meta_ref)?;

        // Parse slab index to capture its section bytes.
        let slab_index_start = cursor.position();
        let slab_index = parse_slab_index(&mut cursor)?;
        let slab_index_end = cursor.position();
        let _ = (slab_index_start, slab_index_end, &slab_index);

        referenced
    };

    // 2. Parse and compact the slab.
    let mut slab_cursor = ManifestCursor::new(slab_bytes);
    let slab_header = parse_slab_header(&mut slab_cursor)?;
    let mut original_count = 0usize;
    let mut kept_drops: Vec<ExtractedDrop> = Vec::new();

    loop {
        let pos = slab_cursor.position();
        let remaining = u64::try_from(
            usize::try_from(slab_header.total_length)
                .unwrap_or(0)
                .saturating_sub(pos),
        )
        .unwrap_or(0);
        if remaining == 0 {
            break;
        }
        let record = parse_drop_record(&mut slab_cursor, &slab_header)?;
        original_count += 1;

        if prefix_end.contains(record.drop_id.as_bytes()) {
            // Read the compressed bytes from the solid window.
            let win_start = slab_cursor.position();
            let win_end = win_start + record.len_in_window as usize;
            if win_end > slab_bytes.len() {
                break;
            }
            kept_drops.push(ExtractedDrop {
                id: *record.drop_id.as_bytes(),
                compressed: slab_bytes[win_start..win_end].to_vec(),
                codec: record.representation.codec,
                plaintext_len: record.plaintext_len,
            });
        } else {
            // Skip the drop's window bytes.
            slab_cursor = ManifestCursor::at_start(
                slab_bytes,
                slab_cursor.position() + record.len_in_window as usize,
            )?;
        }
    }

    let compacted_count = kept_drops.len();
    let reclaimed = original_count.saturating_sub(compacted_count);

    // 3. Build the compacted slab.
    let (new_slab_bytes, new_slab_id) = encode_compacted_slab(&kept_drops);

    // 4. Re-assemble the manifest.
    let new_manifest = reassemble_manifest(manifest_bytes, &new_slab_bytes, &new_slab_id)?;

    Ok(CompactionResult {
        manifest_bytes: new_manifest.bytes,
        merkle_root: new_manifest.merkle_root,
        slab_bytes: Some(new_slab_bytes),
        original_drop_count: original_count,
        compacted_drop_count: compacted_count,
        reclaimed_drops: reclaimed,
    })
}

/// Walk the metadata blob to find all referenced `DropIds`.
fn find_referenced_drops(
    meta_ref: &limnifs_core::MetadataReference,
) -> Result<HashSet<[u8; 32]>, CompactionError> {
    let mut referenced = HashSet::new();
    if let Some(blob_bytes) = &meta_ref.inline_metadata {
        let mut blob_cursor = ManifestCursor::new(blob_bytes);
        let blob = parse_metadata_blob(&mut blob_cursor)?;
        for inode in &blob.inodes {
            if let ContentHandle::SliceMap(slices) = &inode.content_handle {
                for slice in slices {
                    referenced.insert(*slice.drop_id.as_bytes());
                }
            }
        }
    }
    Ok(referenced)
}

/// Encode a compacted slab from extracted drops.
fn encode_compacted_slab(drops: &[ExtractedDrop]) -> (Vec<u8>, SlabId) {
    let mut drop_records = Vec::new();
    let mut solid_window = Vec::new();

    for drop in drops {
        let win_len = u32::try_from(drop.compressed.len()).unwrap_or(0);
        let offset = u32::try_from(solid_window.len()).unwrap_or(0);
        drop_records.extend_from_slice(&drop.id);
        drop_records.extend_from_slice(&drop.plaintext_len.to_le_bytes());
        drop_records.extend_from_slice(&[drop.codec, 0x00, 0x00]); // (codec, aead=0, ec=0)
        drop_records.push(0x00); // solid_window_index
        drop_records.extend_from_slice(&offset.to_le_bytes());
        drop_records.extend_from_slice(&win_len.to_le_bytes());
        solid_window.extend_from_slice(&drop.compressed);
    }

    let slab_content = [&drop_records[..], &solid_window[..]].concat();
    let slab_hash = hash_section(&slab_content);
    let slab_id = SlabId::new(0, slab_hash);

    let total_length = 56 + slab_content.len();
    let mut slab_bytes = Vec::with_capacity(total_length);
    slab_bytes.extend_from_slice(b"LIM1");
    slab_bytes.extend_from_slice(&1u16.to_le_bytes());
    slab_bytes.extend_from_slice(&slab_id.to_bytes());
    slab_bytes.extend_from_slice(&(total_length as u64).to_le_bytes());
    slab_bytes.push(0x00); // ec_descriptor
    slab_bytes.push(0x00); // crypto_hint
    slab_bytes.extend_from_slice(&slab_content);

    (slab_bytes, slab_id)
}

struct ReassembledManifest {
    bytes: Vec<u8>,
    merkle_root: ManifestRoot,
}

/// Re-assemble the manifest with updated slab index.
fn reassemble_manifest(
    source: &[u8],
    _slab_bytes: &[u8],
    slab_id: &SlabId,
) -> Result<ReassembledManifest, CompactionError> {
    let mut cursor = ManifestCursor::new(source);

    // Re-parse and capture each section's raw bytes.
    let header_start = cursor.position();
    let _ = parse_manifest_header(&mut cursor)?;
    let header_end = cursor.position();

    let flags_start = cursor.position();
    let _ = parse_feature_flags_section(&mut cursor)?;
    let flags_end = cursor.position();

    let meta_ref_start = cursor.position();
    let meta_ref = parse_metadata_reference(&mut cursor)?;
    let meta_ref_end = cursor.position();

    // Skip old slab index and history.
    let _ = parse_slab_index(&mut cursor)?;
    let _ = limnifs_core::parse_history(&mut cursor)?;

    // Build new manifest.
    let mut manifest = Vec::new();

    // Copy header + flags + metadata reference verbatim.
    manifest.extend_from_slice(&source[header_start..meta_ref_end]);

    // New slab index.
    let slab_index_start_new = manifest.len();
    manifest.push(SLAB_INDEX_SECTION_VERSION);
    manifest.extend_from_slice(&1u32.to_le_bytes());
    manifest.extend_from_slice(&slab_id.to_bytes());
    manifest.extend_from_slice(&1u32.to_le_bytes());
    let locator = "file:slab-0.bin";
    manifest.extend_from_slice(&u32::try_from(locator.len()).unwrap_or(0).to_le_bytes());
    manifest.extend_from_slice(locator.as_bytes());
    let slab_index_end_new = manifest.len();

    // History.
    let history_start_new = manifest.len();
    manifest.push(HISTORY_SECTION_VERSION);
    manifest.extend_from_slice(&1u32.to_le_bytes());
    manifest.push(0x04); // turnover
    manifest.extend_from_slice(&0u64.to_le_bytes());
    manifest.extend_from_slice(&0u32.to_le_bytes());
    manifest.extend_from_slice(&0u32.to_le_bytes());
    let history_end_new = manifest.len();

    let hashes = SectionHashes {
        metadata: meta_ref.metadata_hash,
        format_header: hash_section(&source[header_start..header_end]),
        feature_flags: hash_section(&source[flags_start..flags_end]),
        metadata_reference: hash_section(&source[meta_ref_start..meta_ref_end]),
        slab_index: hash_section(&manifest[slab_index_start_new..slab_index_end_new]),
        crypto_params: hash_empty_section(),
        ec_params: hash_empty_section(),
        dms_policy: hash_empty_section(),
        delta_linkage: hash_empty_section(),
        history: hash_section(&manifest[history_start_new..history_end_new]),
    };
    let merkle_root = compute_merkle_root(&hashes);

    Ok(ReassembledManifest {
        bytes: manifest,
        merkle_root,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_preserves_referenced_drops() {
        // Build a small image, compact it, verify the compacted
        // image still has the same number of drops.
        let temp =
            std::env::temp_dir().join(format!("limnifs-compaction-test-{}", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp");
        std::fs::write(temp.join("a.txt"), b"hello").expect("write a");
        std::fs::write(temp.join("b.txt"), b"world").expect("write b");

        let artifact = crate::write_directory(&temp).expect("write");
        std::fs::remove_dir_all(&temp).ok();

        let slab_bytes = artifact.slab_bytes.clone().unwrap_or_default();
        if slab_bytes.is_empty() {
            // All files are inline — nothing to compact.
            return;
        }

        let result = compact_image(&artifact.bytes, &slab_bytes).expect("compact");
        assert_eq!(result.original_drop_count, result.compacted_drop_count);
        assert_eq!(result.reclaimed_drops, 0);
    }
}
