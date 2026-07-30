//! Directory node (spec §4.2, `bit-level/34-directory-node.md`).
//!
//! A directory node is a leaf of the deterministic Merkle B-tree that
//! represents a directory's entries. Per pivot D2, v0.1 defines a
//! single layout: the **leaf node** (all entries in one node). Future
//! revisions may introduce internal nodes to scale beyond the leaf
//! ceiling.
//!
//! Entries within a node MUST be lexicographic by name (§1.4). This
//! makes range reads and diff walks deterministic.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::cursor::ManifestCursor;
use crate::error::CoreError;

/// Currently supported node version.
pub const DIRECTORY_NODE_VERSION: u8 = 1;

/// POSIX-equivalent file-type tags used in directory entries.
///
/// These are the same values the [`crate::inode`] module uses for the
/// `S_IFMT` bits, narrowed to the four shapes the directory node
/// allows. Values outside `0x01..=0x04` are reserved.
pub mod entry_type {
    pub const FILE: u8 = 0x01;
    pub const DIRECTORY: u8 = 0x02;
    pub const SYMLINK: u8 = 0x03;
    pub const SPECIAL: u8 = 0x04;
}

/// One directory entry: a name, an inode number, and a type tag.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirEntry {
    pub name: String,
    pub inode_number: u64,
    pub entry_type: u8,
}

/// A parsed directory node (leaf only in v0.1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryNode {
    pub version: u8,
    pub entries: Vec<DirEntry>,
}

/// Parse one directory node from the cursor's current position.
///
/// # Errors
///
/// - [`CoreError::Corrupt`] for structural problems (unsorted entries,
///   duplicate names, empty names, names containing `/` or NUL, an
///   invalid `entry_type`).
/// - [`CoreError::UnsupportedFeature`] for an unknown node version.
/// - [`CoreError::TooShort`] if the cursor underruns.
pub fn parse_directory_node(cursor: &mut ManifestCursor<'_>) -> Result<DirectoryNode, CoreError> {
    let version = cursor.read_u8()?;
    if version != DIRECTORY_NODE_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!("directory node version {version}"),
        });
    }
    let entry_count = cursor.read_u32_le()?;
    let entry_count_us = usize::try_from(entry_count).map_err(|_| CoreError::Corrupt {
        reason: format!("directory node entry_count {entry_count} exceeds usize"),
    })?;
    let mut entries: Vec<DirEntry> = Vec::with_capacity(entry_count_us);
    let mut prev_name: Option<&str> = None;
    for i in 0..entry_count_us {
        let name_len = cursor.read_u32_le()?;
        let name_len_us = usize::try_from(name_len).map_err(|_| CoreError::Corrupt {
            reason: format!("directory node entry {i} name_len {name_len} exceeds usize"),
        })?;
        let name_bytes = cursor.read_n(name_len_us)?;
        if name_bytes.is_empty() {
            return Err(CoreError::Corrupt {
                reason: format!("directory node entry {i}: empty name"),
            });
        }
        if name_bytes.contains(&b'/') {
            return Err(CoreError::Corrupt {
                reason: format!("directory node entry {i}: name contains '/'"),
            });
        }
        if name_bytes.contains(&0) {
            return Err(CoreError::Corrupt {
                reason: format!("directory node entry {i}: name contains NUL byte"),
            });
        }
        let name = std::str::from_utf8(name_bytes).map_err(|_| CoreError::Corrupt {
            reason: format!("directory node entry {i}: name is not valid UTF-8"),
        })?;
        if let Some(prev) = prev_name {
            if prev >= name {
                return Err(CoreError::Corrupt {
                    reason: format!("directory node: entries not sorted ({prev:?} >= {name:?})"),
                });
            }
        }
        let inode_number = cursor.read_u64_le()?;
        let entry_type = cursor.read_u8()?;
        if !matches!(
            entry_type,
            entry_type::FILE | entry_type::DIRECTORY | entry_type::SYMLINK | entry_type::SPECIAL
        ) {
            return Err(CoreError::Corrupt {
                reason: format!("directory node entry {i}: invalid entry_type 0x{entry_type:02X}"),
            });
        }
        prev_name = Some(name);
        entries.push(DirEntry {
            name: name.to_owned(),
            inode_number,
            entry_type,
        });
    }
    Ok(DirectoryNode { version, entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_node(entries: &[(&str, u64, u8)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(DIRECTORY_NODE_VERSION);
        bytes.extend_from_slice(&u32::try_from(entries.len()).unwrap().to_le_bytes());
        for (name, inode_number, entry_type) in entries {
            let name_bytes = name.as_bytes();
            let name_len = u32::try_from(name_bytes.len()).unwrap();
            bytes.extend_from_slice(&name_len.to_le_bytes());
            bytes.extend_from_slice(name_bytes);
            bytes.extend_from_slice(&inode_number.to_le_bytes());
            bytes.push(*entry_type);
        }
        bytes
    }

    #[test]
    fn parses_sorted_directory_node() {
        let bytes = encode_node(&[
            ("README.md", 3, entry_type::FILE),
            ("bin", 1, entry_type::DIRECTORY),
            ("hello.txt", 2, entry_type::FILE),
        ]);
        let mut cursor = ManifestCursor::new(&bytes);
        let node = parse_directory_node(&mut cursor).expect("parses");
        assert_eq!(node.version, 1);
        assert_eq!(node.entries.len(), 3);
        assert_eq!(node.entries[0].name, "README.md");
        assert_eq!(node.entries[1].name, "bin");
        assert_eq!(node.entries[2].name, "hello.txt");
        assert_eq!(node.entries[1].entry_type, entry_type::DIRECTORY);
    }

    #[test]
    fn parses_empty_directory() {
        let bytes = encode_node(&[]);
        let mut cursor = ManifestCursor::new(&bytes);
        let node = parse_directory_node(&mut cursor).expect("empty node parses");
        assert_eq!(node.version, 1);
        assert!(node.entries.is_empty());
        assert_eq!(cursor.position(), bytes.len());
    }

    #[test]
    fn rejects_unsorted_entries() {
        // "z" comes before "a" — out of order.
        let bytes = encode_node(&[("z", 1, entry_type::FILE), ("a", 2, entry_type::FILE)]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_directory_node(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("not sorted"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_name() {
        // Manually build a node with a zero-length name to bypass the
        // encoder's `&str` typing.
        let mut bytes = Vec::new();
        bytes.push(DIRECTORY_NODE_VERSION);
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // name_len = 0
        bytes.extend_from_slice(&1u64.to_le_bytes()); // inode_number
        bytes.push(entry_type::FILE);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_directory_node(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("empty name"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_name_with_slash() {
        let bytes = encode_node(&[("foo/bar", 1, entry_type::FILE)]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_directory_node(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("'/'"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_invalid_entry_type() {
        let mut bytes = Vec::new();
        bytes.push(DIRECTORY_NODE_VERSION);
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes()); // name_len
        bytes.extend_from_slice(b"x");
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.push(0x05); // reserved
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_directory_node(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("invalid entry_type"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_node_version() {
        let mut bytes = vec![0x07]; // version 7
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_directory_node(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains('7'), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_too_short_for_prefix() {
        let bytes = [DIRECTORY_NODE_VERSION];
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_directory_node(&mut cursor) {
            Err(CoreError::TooShort { .. }) => {}
            other => panic!("expected TooShort, got {other:?}"),
        }
    }
}
