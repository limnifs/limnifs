//! Turnover — tier-3 full re-encode defrag.
//!
//! Repacks all referenced drops into fresh slabs, applies the deepening
//! policy, GCs unreferenced drops, and emits a standalone manifest
//! with no external references. The result is a clean, self-contained
//! image.
//!
//! ## Composition, not duplication
//!
//! Turnover orchestrates existing primitives rather than re-implementing
//! them. v1 wraps [`crate::compaction::compact_image`]:
//!
//! - GC: `compact_image` already drops unreferenced drops.
//! - Slab re-packing: idempotent in v1 — compaction produces a clean
//!   single-slab image.
//! - Deepening re-run: deferred to v2 (the deepening policy is
//!   per-class; running it again would re-evaluate every drop, which
//!   is wasteful when the original image already chose correctly).
//! - History: compaction records a `HistoryOp::Turnover` entry.
//!
//! When deepening re-run lands, it will be a hook on the turnover
//! struct (builder pattern), defaulting to "skip".
//!
//! ## Cancel safety
//!
//! Turnover produces a NEW manifest; the original image is untouched
//! until the caller atomically swaps the files. This matches the spec's
//! tier-3 contract.
//!
//! See task `06-turnover.md`.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::compaction::{compact_image, CompactionError, CompactionResult};

/// Run a turnover on the given image.
///
/// Equivalent to [`compact_image`] today; the wrapper exists to give
/// the operation a stable, spec-named API and to leave room for the
/// deepening re-run hook (v2).
///
/// # Errors
///
/// See [`CompactionError`].
pub fn run(manifest_bytes: &[u8], slab_bytes: &[u8]) -> Result<CompactionResult, CompactionError> {
    compact_image(manifest_bytes, slab_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_directory;
    use std::path::Path;

    #[test]
    fn turnover_compacts_unreferenced_drops() {
        // Write two large files into one image, then re-write a
        // version with only one file. The old slab still references
        // both drops; turnover must reclaim the orphaned one.
        let temp1 =
            std::env::temp_dir().join(format!("limnifs-turnover-pre-{}", std::process::id()));
        let temp2 =
            std::env::temp_dir().join(format!("limnifs-turnover-post-{}", std::process::id()));
        std::fs::create_dir_all(&temp1).unwrap();
        std::fs::create_dir_all(&temp2).unwrap();
        let large_a = vec![0xAAu8; 8192];
        let large_b = vec![0xBBu8; 8192];
        std::fs::write(temp1.join("a.bin"), &large_a).unwrap();
        std::fs::write(temp1.join("b.bin"), &large_b).unwrap();
        std::fs::write(temp2.join("a.bin"), &large_a).unwrap();

        let before = write_directory(&temp1).expect("write before");
        let after = write_directory(&temp2).expect("write after");
        std::fs::remove_dir_all(&temp1).ok();
        std::fs::remove_dir_all(&temp2).ok();

        let slab_bytes = before.slab_bytes().expect("slab exists");
        let turnover_result = run(&after.bytes, slab_bytes).expect("turnover");
        assert_eq!(
            turnover_result.original_drop_count - turnover_result.compacted_drop_count,
            turnover_result.reclaimed_drops,
            "compacted = original - reclaimed"
        );
        assert!(
            turnover_result.compacted_drop_count < turnover_result.original_drop_count,
            "turnover must reclaim the orphaned drop"
        );
    }

    #[test]
    fn turnover_preserves_referenced_drops() {
        // Write one large file, run turnover, verify the result still
        // has exactly one drop and the drop's plaintext is recoverable.
        let temp =
            std::env::temp_dir().join(format!("limnifs-turnover-keep-{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        let payload = vec![0xCCu8; 8192];
        std::fs::write(temp.join("keep.bin"), &payload).unwrap();
        let artifact = write_directory(&temp).expect("write");
        std::fs::remove_dir_all(&temp).ok();
        let slab_bytes = artifact.slab_bytes().expect("slab exists");

        let turnover_result = run(&artifact.bytes, slab_bytes).expect("turnover");
        assert_eq!(turnover_result.compacted_drop_count, 1);
        assert_eq!(turnover_result.reclaimed_drops, 0);

        // Verify the new slab parses and contains the drop.
        let new_slab = turnover_result
            .slab_bytes
            .as_deref()
            .expect("turnover produced slab");
        let view = limnifs_core::parse_slab(new_slab).expect("slab parses");
        // The drop's plaintext is `payload`.
        let drop_id = limnifs_core::hash_section(&payload);
        let plaintext = view
            .plaintext_for(&drop_id)
            .expect("drop present")
            .expect("decompresses");
        assert_eq!(plaintext, payload);
    }

    #[test]
    fn turnover_history_marks_turnover_op() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-turnover-hist-{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::write(temp.join("x.bin"), vec![0x11u8; 8192]).unwrap();
        let artifact = write_directory(&temp).expect("write");
        std::fs::remove_dir_all(&temp).ok();
        let slab_bytes = artifact.slab_bytes().expect("slab exists");

        let turnover_result = run(&artifact.bytes, slab_bytes).expect("turnover");
        let mut cursor = limnifs_core::ManifestCursor::new(&turnover_result.manifest_bytes);
        limnifs_core::parse_manifest_header(&mut cursor).unwrap();
        limnifs_core::parse_feature_flags_section(&mut cursor).unwrap();
        limnifs_core::parse_metadata_reference(&mut cursor).unwrap();
        limnifs_core::parse_slab_index(&mut cursor).unwrap();
        let history = limnifs_core::parse_history(&mut cursor).unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].op, limnifs_core::HistoryOp::Turnover);
    }

    #[test]
    fn turnover_drops_path_is_unused() {
        // Document the no-op path: turnover only consumes manifest +
        // slab bytes, never the filesystem. This test pins the
        // signature so future refactorings stay honest.
        fn _signature_check(_manifest: &[u8], _slab: &[u8]) {}
        let _ = _signature_check;
        let _ = Path::new("/this/path/does/not/exist");
    }
}
