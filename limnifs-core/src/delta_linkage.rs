//! Delta linkage section (spec §5.8, `bit-level/45-delta-linkage.md`).
//!
//! Identifies this image as a delta against a parent image and
//! records the tree operations (Add/Remove/Replace) that transform
//! the parent's filesystem tree into this image's tree. Absent for
//! non-delta images.

use crate::cursor::ManifestCursor;
use crate::error::CoreError;
use limnifs_format::ManifestRoot;

/// Current layout version of this section.
pub const DELTA_LINKAGE_SECTION_VERSION: u8 = 1;

/// Width of the fixed prefix: `version` + `base_root` + `tree_op_count`.
const PREFIX_LEN: usize = 1 + 32 + 4;

/// Minimum size of a single tree op: `op_type` + `path_len` + 1-byte path.
const MIN_TREE_OP_LEN: usize = 1 + 4 + 1;

/// Tree operation kind per spec §5.8 / §20.2.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum TreeOpKind {
    Add = 0x01,
    Remove = 0x02,
    Replace = 0x03,
}

impl TreeOpKind {
    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(Self::Add),
            0x02 => Some(Self::Remove),
            0x03 => Some(Self::Replace),
            _ => None,
        }
    }

    /// True iff this op carries an `inode_number` field.
    #[must_use]
    pub const fn has_inode_number(self) -> bool {
        !matches!(self, Self::Remove)
    }
}

/// One tree operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeOp {
    pub kind: TreeOpKind,
    pub path: String,
    /// Present for Add and Replace; absent for Remove.
    pub inode_number: Option<u64>,
}

/// Parsed delta linkage section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeltaLinkage {
    pub base_root: ManifestRoot,
    pub tree_ops: Vec<TreeOp>,
}

/// Parse the delta linkage section from the cursor's current position.
///
/// # Errors
///
/// - [`CoreError::UnsupportedFeature`] if `section_version` is not 1
///   or any tree op's `op_type` is reserved/extended.
/// - [`CoreError::Corrupt`] if a path is empty, contains NUL bytes,
///   or has empty components (double-slash); or if an Add/Replace op
///   is missing its `inode_number`.
/// - [`CoreError::TooShort`] if the cursor underruns.
pub fn parse_delta_linkage(cursor: &mut ManifestCursor<'_>) -> Result<DeltaLinkage, CoreError> {
    let section_version = cursor.read_u8()?;
    if section_version != DELTA_LINKAGE_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "delta_linkage section version {section_version} (supported: {DELTA_LINKAGE_SECTION_VERSION})"
            ),
        });
    }
    let base_root_bytes = cursor.read_n(32)?;
    let mut base_root_arr = [0u8; 32];
    base_root_arr.copy_from_slice(base_root_bytes);
    let base_root = ManifestRoot::from_bytes(base_root_arr);

    let raw_count = cursor.read_u32_le()?;
    let op_count = usize::try_from(raw_count).map_err(|_| CoreError::Corrupt {
        reason: format!("delta_linkage tree_op_count {raw_count} exceeds usize"),
    })?;
    let min_total = op_count
        .checked_mul(MIN_TREE_OP_LEN)
        .ok_or_else(|| CoreError::Corrupt {
            reason: format!("delta_linkage tree_op_count {op_count} overflows usize"),
        })?;
    if cursor.remaining_len() < min_total {
        return Err(CoreError::TooShort {
            have: cursor.remaining_len(),
            need: min_total,
        });
    }

    let mut tree_ops = Vec::with_capacity(op_count);
    for index in 0..op_count {
        let op_byte = cursor.read_u8()?;
        if op_byte == 0xFF {
            return Err(CoreError::UnsupportedFeature {
                feature: format!("delta_linkage tree_op {index}: op_type 0xFF (extended, post-v1)"),
            });
        }
        let kind = TreeOpKind::from_byte(op_byte).ok_or_else(|| CoreError::UnsupportedFeature {
            feature: format!(
                "delta_linkage tree_op {index}: op_type 0x{op_byte:02X} is reserved (not in 0x01..0x03)"
            ),
        })?;
        let path_len = cursor.read_u32_le()?;
        let path_len_us = usize::try_from(path_len).map_err(|_| CoreError::Corrupt {
            reason: format!("delta_linkage tree_op {index}: path_len {path_len} exceeds usize"),
        })?;
        if path_len_us == 0 {
            return Err(CoreError::Corrupt {
                reason: format!("delta_linkage tree_op {index}: empty path"),
            });
        }
        let path_bytes = cursor.read_n(path_len_us)?;
        let path = std::str::from_utf8(path_bytes).map_err(|_| CoreError::Corrupt {
            reason: format!("delta_linkage tree_op {index}: path is not valid UTF-8"),
        })?;
        if path.contains('\0') {
            return Err(CoreError::Corrupt {
                reason: format!("delta_linkage tree_op {index}: path contains NUL bytes"),
            });
        }
        if path.contains("//") {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "delta_linkage tree_op {index}: path has empty component (double-slash)"
                ),
            });
        }
        let inode_number = if kind.has_inode_number() {
            Some(cursor.read_u64_le()?)
        } else {
            None
        };
        tree_ops.push(TreeOp {
            kind,
            path: path.to_owned(),
            inode_number,
        });
    }
    let _ = PREFIX_LEN; // documented constant; nothing to emit
    Ok(DeltaLinkage {
        base_root,
        tree_ops,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tree_op_bytes(kind: u8, path: &str, inode: Option<u64>) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(kind);
        let path_bytes = path.as_bytes();
        let len = u32::try_from(path_bytes.len()).expect("path fits u32");
        bytes.extend_from_slice(&len.to_le_bytes());
        bytes.extend_from_slice(path_bytes);
        if let Some(inode) = inode {
            bytes.extend_from_slice(&inode.to_le_bytes());
        }
        bytes
    }

    fn make_delta_bytes(version: u8, base_root: [u8; 32], ops: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(version);
        bytes.extend_from_slice(&base_root);
        let count = u32::try_from(ops.len()).expect("count fits u32");
        bytes.extend_from_slice(&count.to_le_bytes());
        for op in ops {
            bytes.extend_from_slice(op);
        }
        bytes
    }

    fn sample_root() -> [u8; 32] {
        [0xBB; 32]
    }

    #[test]
    fn parses_add_and_remove() {
        let bytes = make_delta_bytes(
            DELTA_LINKAGE_SECTION_VERSION,
            sample_root(),
            &[
                make_tree_op_bytes(0x01, "usr/bin/newtool", Some(42)),
                make_tree_op_bytes(0x02, "usr/bin/oldtool", None),
            ],
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_delta_linkage(&mut cursor).expect("add+remove parses");
        assert_eq!(parsed.tree_ops.len(), 2);
        assert_eq!(parsed.tree_ops[0].kind, TreeOpKind::Add);
        assert_eq!(parsed.tree_ops[0].path, "usr/bin/newtool");
        assert_eq!(parsed.tree_ops[0].inode_number, Some(42));
        assert_eq!(parsed.tree_ops[1].kind, TreeOpKind::Remove);
        assert_eq!(parsed.tree_ops[1].path, "usr/bin/oldtool");
        assert_eq!(parsed.tree_ops[1].inode_number, None);
        assert_eq!(cursor.position(), bytes.len());
    }

    #[test]
    fn parses_replace() {
        let bytes = make_delta_bytes(
            DELTA_LINKAGE_SECTION_VERSION,
            sample_root(),
            &[make_tree_op_bytes(0x03, "etc/config", Some(99))],
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_delta_linkage(&mut cursor).expect("replace parses");
        assert_eq!(parsed.tree_ops[0].kind, TreeOpKind::Replace);
        assert_eq!(parsed.tree_ops[0].inode_number, Some(99));
    }

    #[test]
    fn parses_zero_ops() {
        let bytes = make_delta_bytes(DELTA_LINKAGE_SECTION_VERSION, sample_root(), &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_delta_linkage(&mut cursor).expect("zero ops parses");
        assert!(parsed.tree_ops.is_empty());
    }

    #[test]
    fn rejects_unknown_section_version() {
        let bytes = make_delta_bytes(7, sample_root(), &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_delta_linkage(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("version 7"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_reserved_op_type() {
        let bytes = make_delta_bytes(
            DELTA_LINKAGE_SECTION_VERSION,
            sample_root(),
            &[make_tree_op_bytes(0x05, "x", None)],
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_delta_linkage(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("reserved"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_extended_op_type() {
        let bytes = make_delta_bytes(
            DELTA_LINKAGE_SECTION_VERSION,
            sample_root(),
            &[make_tree_op_bytes(0xFF, "x", None)],
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_delta_linkage(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("0xFF"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_path() {
        let mut bytes = Vec::new();
        bytes.push(DELTA_LINKAGE_SECTION_VERSION);
        bytes.extend_from_slice(&sample_root());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.push(0x01); // Add
        bytes.extend_from_slice(&0u32.to_le_bytes()); // path_len = 0
        bytes.extend_from_slice(&42u64.to_le_bytes()); // inode_number for Add
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_delta_linkage(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("empty path"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_double_slash_path() {
        let bytes = make_delta_bytes(
            DELTA_LINKAGE_SECTION_VERSION,
            sample_root(),
            &[make_tree_op_bytes(0x01, "a//b", Some(1))],
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_delta_linkage(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("double-slash"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_truncated_prefix() {
        let bytes = [DELTA_LINKAGE_SECTION_VERSION, 0, 0];
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_delta_linkage(&mut cursor) {
            Err(CoreError::TooShort { .. }) => {}
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn tree_op_kind_has_inode_number_predicate() {
        assert!(TreeOpKind::Add.has_inode_number());
        assert!(TreeOpKind::Replace.has_inode_number());
        assert!(!TreeOpKind::Remove.has_inode_number());
    }
}
