//! Delta builder — computes tree operations between two images.
//!
//! Given a parent image and a child image, produces the tree
//! operations (Add / Remove / Replace) that transform the parent's
//! filesystem tree into the child's. The output is a delta linkage
//! section (spec §5.8) that can be appended to a manifest.
//!
//! ## Algorithm
//!
//! 1. Parse both images' metadata blobs to obtain their directory trees.
//! 2. Walk both trees simultaneously from their root inodes.
//! 3. At each directory, compare entry sets:
//!    - Entry only in child → emit `Add(path, child_inode)`.
//!    - Entry only in parent → emit `Remove(path)`.
//!    - Entry in both with different inode content → emit
//!      `Replace(path, child_inode)`.
//!    - Entry in both with same content → recurse if both are
//!      directories, otherwise no-op.
//! 4. Collect all operations in deterministic order (sorted by path).
//!
//! Content identity is determined by the inode's content handle:
//! - Directories: BLAKE3 hash of the directory node bytes.
//! - Files: the drop ID (for drop-backed) or BLAKE3 of inline data.
//! - Other types: compared by full inode equality.
//!
//! Two inodes with the same content but different inode numbers are
//! NOT considered different (they map to the same bytes on disk).

use std::collections::BTreeMap;
use std::path::Path;

use limnifs_core::delta_linkage::{TreeOp, TreeOpKind};
use limnifs_core::{
    parse_manifest_header, parse_metadata_blob, parse_metadata_reference, ContentHandle, CoreError,
    Inode, ManifestCursor, MetadataBlob,
};

/// Error during delta computation.
#[derive(Debug)]
pub enum DeltaError {
    Core(CoreError),
    Io(std::io::Error),
}

impl std::fmt::Display for DeltaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core(e) => write!(f, "format error: {e}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for DeltaError {}

impl From<CoreError> for DeltaError {
    fn from(e: CoreError) -> Self {
        Self::Core(e)
    }
}

impl From<std::io::Error> for DeltaError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<crate::WriteError> for DeltaError {
    fn from(e: crate::WriteError) -> Self {
        match e {
            crate::WriteError::Io(io) => Self::Io(io),
            crate::WriteError::UnsupportedFileType { path, kind } => Self::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("unsupported file type ({kind}): {}", path.display()),
            )),
        }
    }
}

/// Result of computing a delta: the tree operations + the parent's
/// `ManifestRoot` (to embed in the delta linkage section's `base_root`).
#[derive(Clone, Debug)]
pub struct DeltaArtifact {
    pub tree_ops: Vec<TreeOp>,
    pub base_root: [u8; 32],
}

impl DeltaArtifact {
    /// Encode the delta as a delta linkage section (spec §5.8).
    ///
    /// # Panics
    ///
    /// Panics if the tree-op count or any path length exceeds `u32`.
    /// Both are bounded by the metadata blob's inode count and path
    /// lengths, which the writer validates upstream.
    #[must_use]
    pub fn to_section_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(1u8); // section_version
        bytes.extend_from_slice(&self.base_root);
        let count = u32::try_from(self.tree_ops.len()).expect("tree_op_count fits u32");
        bytes.extend_from_slice(&count.to_le_bytes());
        for op in &self.tree_ops {
            bytes.push(op.kind as u8);
            let path_bytes = op.path.as_bytes();
            let path_len = u32::try_from(path_bytes.len()).expect("path fits u32");
            bytes.extend_from_slice(&path_len.to_le_bytes());
            bytes.extend_from_slice(path_bytes);
            if let Some(inode_number) = op.inode_number {
                bytes.extend_from_slice(&inode_number.to_le_bytes());
            }
        }
        bytes
    }
}

/// Compute the delta between a parent image and a child image. Both
/// are `.lim` manifest files on disk.
///
/// # Errors
///
/// Returns [`DeltaError`] if either image fails to parse or is not
/// an inlined-metadata image.
pub fn compute_delta(parent_path: &Path, child_path: &Path) -> Result<DeltaArtifact, DeltaError> {
    let parent_bytes = std::fs::read(parent_path)?;
    let child_bytes = std::fs::read(child_path)?;
    compute_delta_from_bytes(&parent_bytes, &child_bytes)
}

/// Same as [`compute_delta`] but takes raw manifest bytes instead of
/// file paths. Useful for testing.
///
/// # Errors
///
/// Returns [`DeltaError`] if either image fails to parse or is not
/// an inlined-metadata image.
///
/// # Panics
///
/// Panics if either image's root inode is missing after validation
/// (cannot happen — `load_image` validates it before returning).
pub fn compute_delta_from_bytes(
    parent_bytes: &[u8],
    child_bytes: &[u8],
) -> Result<DeltaArtifact, DeltaError> {
    let (parent_blob, parent_root_number, parent_merkle_root) = load_image(parent_bytes)?;
    let (child_blob, child_root_number, _) = load_image(child_bytes)?;

    let parent_root_inode = parent_blob
        .inode_by_number(parent_root_number)
        .expect("load_image validates root inode exists");
    let child_root_inode = child_blob
        .inode_by_number(child_root_number)
        .expect("load_image validates root inode exists");

    let mut ops = Vec::new();
    diff_directory(
        &parent_blob,
        &child_blob,
        parent_root_inode,
        child_root_inode,
        "",
        &mut ops,
    );
    ops.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(DeltaArtifact {
        tree_ops: ops,
        base_root: parent_merkle_root,
    })
}

/// Load an image: parse manifest header, metadata reference, extract
/// the inlined metadata blob, find the root inode number, and read
/// the Merkle root from the manifest.
fn load_image(bytes: &[u8]) -> Result<(MetadataBlob, u64, [u8; 32]), DeltaError> {
    let mut cursor = ManifestCursor::new(bytes);
    let header = parse_manifest_header(&mut cursor)?;
    let _ = header;
    let _ = limnifs_core::parse_feature_flags_section(&mut cursor)?;
    let meta_ref = parse_metadata_reference(&mut cursor)?;
    let blob_bytes = meta_ref.inline_metadata.as_deref().ok_or_else(|| {
        DeltaError::Core(CoreError::Corrupt {
            reason: "delta builder requires inlined metadata".into(),
        })
    })?;
    let mut blob_cursor = ManifestCursor::new(blob_bytes);
    let blob = parse_metadata_blob(&mut blob_cursor)?;

    let root_number = blob.root_inode_number().ok_or_else(|| {
        DeltaError::Core(CoreError::Corrupt {
            reason: "metadata blob: could not identify a unique root directory inode".into(),
        })
    })?;
    if blob.inode_by_number(root_number).is_none() {
        return Err(DeltaError::Core(CoreError::Corrupt {
            reason: format!("metadata blob: root inode {root_number} missing"),
        }));
    }

    let mut merkle_root = [0u8; 32];
    // The delta builder needs the parent's ManifestRoot. For now we
    // hash the full manifest as a proxy. The caller should supply the
    // correct base_root if precision matters (e.g. via verify).
    limnifs_core::hash_section(bytes).clone_into(&mut merkle_root);

    Ok((blob, root_number, merkle_root))
}

/// Recursively diff two directory inodes, appending `TreeOps` for
/// every difference found.
fn diff_directory(
    parent_blob: &MetadataBlob,
    child_blob: &MetadataBlob,
    parent_inode: &Inode,
    child_inode: &Inode,
    path: &str,
    ops: &mut Vec<TreeOp>,
) {
    let p_hash = match &parent_inode.content_handle {
        ContentHandle::Directory(h) => *h,
        _ => return,
    };
    let c_hash = match &child_inode.content_handle {
        ContentHandle::Directory(h) => *h,
        _ => return,
    };

    let Some(p_node) = parent_blob.dir_node_by_hash(&p_hash) else {
        return;
    };
    let Some(c_node) = child_blob.dir_node_by_hash(&c_hash) else {
        return;
    };

    // Build lookup maps: name → inode_number.
    let parent_entries: BTreeMap<&str, u64> = p_node
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.inode_number))
        .collect();
    let child_entries: BTreeMap<&str, u64> = c_node
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.inode_number))
        .collect();

    for (name, child_inum) in &child_entries {
        let child_path = if path.is_empty() {
            (*name).to_owned()
        } else {
            format!("{path}/{name}")
        };
        match parent_entries.get(*name) {
            None => {
                // Entry only in child → Add.
                ops.push(TreeOp {
                    kind: TreeOpKind::Add,
                    path: child_path,
                    inode_number: Some(*child_inum),
                });
            }
            Some(parent_inum) => {
                // Entry in both — compare content.
                let parent_child_inode = parent_blob.inode_by_number(*parent_inum);
                let child_child_inode = child_blob.inode_by_number(*child_inum);
                if let (Some(pci), Some(cci)) = (parent_child_inode, child_child_inode) {
                    if pci.is_directory() && cci.is_directory() {
                        // Always recurse into matching directories to
                        // find per-entry deltas. Replaces are only
                        // emitted for files, not directories — a
                        // directory "change" is expressed as Adds /
                        // Removes / Replaces on its children.
                        diff_directory(parent_blob, child_blob, pci, cci, &child_path, ops);
                    } else if !inodes_equal(pci, cci) {
                        ops.push(TreeOp {
                            kind: TreeOpKind::Replace,
                            path: child_path,
                            inode_number: Some(*child_inum),
                        });
                    }
                }
            }
        }
    }

    for name in parent_entries.keys() {
        if !child_entries.contains_key(*name) {
            let parent_path = if path.is_empty() {
                (*name).to_owned()
            } else {
                format!("{path}/{name}")
            };
            ops.push(TreeOp {
                kind: TreeOpKind::Remove,
                path: parent_path,
                inode_number: None,
            });
        }
    }
}

/// Determine if two inodes have the same content (identity). Two
/// inodes are identical iff their content handles produce the same
/// bytes — regardless of their inode numbers.
fn inodes_equal(a: &Inode, b: &Inode) -> bool {
    if a.file_type() != b.file_type() {
        return false;
    }
    match (&a.content_handle, &b.content_handle) {
        (ContentHandle::InlineData(da), ContentHandle::InlineData(db)) => da == db,
        (ContentHandle::SliceMap(sa), ContentHandle::SliceMap(sb)) => {
            // Compare by the drop IDs each slice references. Two
            // files are identical iff their slice maps reference the
            // same drops in the same order.
            if sa.len() != sb.len() {
                return false;
            }
            sa.iter()
                .zip(sb.iter())
                .all(|(x, y)| x.drop_id.as_bytes() == y.drop_id.as_bytes())
        }
        (ContentHandle::Directory(ha), ContentHandle::Directory(hb)) => ha == hb,
        (ContentHandle::Symlink(ta), ContentHandle::Symlink(tb)) => ta == tb,
        (ContentHandle::Device(da), ContentHandle::Device(db)) => da == db,
        (ContentHandle::Pipe(pa), ContentHandle::Pipe(pb)) => pa == pb,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_image(dir: &Path) -> Result<Vec<u8>, DeltaError> {
        let artifact = crate::write_directory(dir)?;
        Ok(artifact.bytes)
    }

    #[test]
    fn identical_images_produce_empty_delta() {
        let temp = std::env::temp_dir().join(format!(
            "limnifs-delta-test-{}-identical",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp).expect("create temp");
        std::fs::write(temp.join("a.txt"), b"aaa").expect("write a");
        let bytes = write_image(&temp).expect("write image");
        std::fs::remove_dir_all(&temp).ok();

        let delta = compute_delta_from_bytes(&bytes, &bytes).expect("delta computes");
        assert!(
            delta.tree_ops.is_empty(),
            "identical images should have no ops"
        );
    }

    #[test]
    fn added_file_produces_add_op() {
        let parent_dir = std::env::temp_dir().join(format!(
            "limnifs-delta-test-{}-parent-add",
            std::process::id()
        ));
        let child_dir = std::env::temp_dir().join(format!(
            "limnifs-delta-test-{}-child-add",
            std::process::id()
        ));
        std::fs::create_dir_all(&parent_dir).expect("create parent");
        std::fs::create_dir_all(&child_dir).expect("create child");
        std::fs::write(parent_dir.join("a.txt"), b"aaa").expect("write a");
        std::fs::write(child_dir.join("a.txt"), b"aaa").expect("copy a");
        std::fs::write(child_dir.join("b.txt"), b"bbb").expect("write b");

        let parent_bytes = write_image(&parent_dir).expect("parent");
        let child_bytes = write_image(&child_dir).expect("child");
        std::fs::remove_dir_all(&parent_dir).ok();
        std::fs::remove_dir_all(&child_dir).ok();

        let delta = compute_delta_from_bytes(&parent_bytes, &child_bytes).expect("delta");
        let adds: Vec<&TreeOp> = delta
            .tree_ops
            .iter()
            .filter(|op| op.kind == TreeOpKind::Add)
            .collect();
        assert_eq!(adds.len(), 1, "expected exactly one Add op");
        assert_eq!(adds[0].path, "b.txt");
        assert!(adds[0].inode_number.is_some());
    }

    #[test]
    fn removed_file_produces_remove_op() {
        let parent_dir = std::env::temp_dir().join(format!(
            "limnifs-delta-test-{}-parent-rm",
            std::process::id()
        ));
        let child_dir = std::env::temp_dir().join(format!(
            "limnifs-delta-test-{}-child-rm",
            std::process::id()
        ));
        std::fs::create_dir_all(&parent_dir).expect("create parent");
        std::fs::create_dir_all(&child_dir).expect("create child");
        std::fs::write(parent_dir.join("a.txt"), b"aaa").expect("write a");
        std::fs::write(parent_dir.join("b.txt"), b"bbb").expect("write b");
        std::fs::write(child_dir.join("a.txt"), b"aaa").expect("copy a");

        let parent_bytes = write_image(&parent_dir).expect("parent");
        let child_bytes = write_image(&child_dir).expect("child");
        std::fs::remove_dir_all(&parent_dir).ok();
        std::fs::remove_dir_all(&child_dir).ok();

        let delta = compute_delta_from_bytes(&parent_bytes, &child_bytes).expect("delta");
        let removes: Vec<&TreeOp> = delta
            .tree_ops
            .iter()
            .filter(|op| op.kind == TreeOpKind::Remove)
            .collect();
        assert_eq!(removes.len(), 1, "expected exactly one Remove op");
        assert_eq!(removes[0].path, "b.txt");
        assert!(removes[0].inode_number.is_none());
    }

    #[test]
    fn modified_file_produces_replace_op() {
        let parent_dir = std::env::temp_dir().join(format!(
            "limnifs-delta-test-{}-parent-mod",
            std::process::id()
        ));
        let child_dir = std::env::temp_dir().join(format!(
            "limnifs-delta-test-{}-child-mod",
            std::process::id()
        ));
        std::fs::create_dir_all(&parent_dir).expect("create parent");
        std::fs::create_dir_all(&child_dir).expect("create child");
        std::fs::write(parent_dir.join("a.txt"), b"original").expect("write a");
        std::fs::write(child_dir.join("a.txt"), b"modified").expect("write a");

        let parent_bytes = write_image(&parent_dir).expect("parent");
        let child_bytes = write_image(&child_dir).expect("child");
        std::fs::remove_dir_all(&parent_dir).ok();
        std::fs::remove_dir_all(&child_dir).ok();

        let delta = compute_delta_from_bytes(&parent_bytes, &child_bytes).expect("delta");
        let replaces: Vec<&TreeOp> = delta
            .tree_ops
            .iter()
            .filter(|op| op.kind == TreeOpKind::Replace)
            .collect();
        assert_eq!(replaces.len(), 1, "expected exactly one Replace op");
        assert_eq!(replaces[0].path, "a.txt");
        assert!(replaces[0].inode_number.is_some());
    }

    #[test]
    fn delta_section_bytes_round_trip() {
        let ops = vec![
            TreeOp {
                kind: TreeOpKind::Add,
                path: "new.txt".into(),
                inode_number: Some(42),
            },
            TreeOp {
                kind: TreeOpKind::Remove,
                path: "old.txt".into(),
                inode_number: None,
            },
        ];
        let artifact = DeltaArtifact {
            tree_ops: ops,
            base_root: [0xAB; 32],
        };
        let bytes = artifact.to_section_bytes();

        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = limnifs_core::delta_linkage::parse_delta_linkage(&mut cursor).expect("parses");
        assert_eq!(parsed.tree_ops.len(), 2);
        assert_eq!(parsed.tree_ops[0].path, "new.txt");
        assert_eq!(parsed.tree_ops[0].inode_number, Some(42));
        assert_eq!(parsed.tree_ops[1].path, "old.txt");
        assert_eq!(parsed.tree_ops[1].inode_number, None);
    }

    #[test]
    fn subdirectory_changes_produce_nested_ops() {
        let parent_dir = std::env::temp_dir().join(format!(
            "limnifs-delta-test-{}-parent-sub",
            std::process::id()
        ));
        let child_dir = std::env::temp_dir().join(format!(
            "limnifs-delta-test-{}-child-sub",
            std::process::id()
        ));
        std::fs::create_dir_all(parent_dir.join("sub")).expect("create parent/sub");
        std::fs::create_dir_all(child_dir.join("sub")).expect("create child/sub");
        std::fs::write(parent_dir.join("sub").join("a.txt"), b"a").expect("write a");
        std::fs::write(parent_dir.join("root.txt"), b"root").expect("write root");
        std::fs::write(child_dir.join("sub").join("a.txt"), b"a").expect("copy a");
        std::fs::write(child_dir.join("sub").join("b.txt"), b"b").expect("write b");
        std::fs::write(child_dir.join("root.txt"), b"root").expect("copy root");

        let parent_bytes = write_image(&parent_dir).expect("parent");
        let child_bytes = write_image(&child_dir).expect("child");
        std::fs::remove_dir_all(&parent_dir).ok();
        std::fs::remove_dir_all(&child_dir).ok();

        let delta = compute_delta_from_bytes(&parent_bytes, &child_bytes).expect("delta");
        let add_paths: Vec<&str> = delta
            .tree_ops
            .iter()
            .filter(|op| op.kind == TreeOpKind::Add)
            .map(|op| op.path.as_str())
            .collect();
        assert!(
            add_paths.contains(&"sub/b.txt"),
            "expected sub/b.txt in adds, got {add_paths:?}"
        );
    }
}
