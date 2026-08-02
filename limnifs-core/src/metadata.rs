//! Metadata blob (spec §4.1 + §4.2, `bit-level/33-inode.md` and
//! `bit-level/34-directory-node.md`).
//!
//! The metadata blob is the layer-2 payload of a `LimniFS` image: every
//! inode and every directory node, packed contiguously. The
//! [`crate::metadata_reference`] section's `inline_metadata` field (or
//! the locators it carries) is the entry point; once the bytes are in
//! hand, this parser turns them into typed values.
//!
//! ## Layout (v0.1)
//!
//! ```text
//! +--------------------------------------------------+
//! | inode_count    : u32 LE                           |  offset 0
//! +--------------------------------------------------+
//! | inodes[]       : inode_count × Inode             |  offset 4
//! +--------------------------------------------------+
//! | dir_node_count : u32 LE                           |  variable
//! +--------------------------------------------------+
//! | dir_nodes[]    : dir_node_count × DirectoryNode  |  variable
//! +--------------------------------------------------+
//! ```
//!
//! The order within each list is the writer's choice. Readers MUST NOT
//! rely on a particular ordering; instead, they look up entries by
//! `inode_number` or by `btree_node_hash`.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::cursor::ManifestCursor;
use crate::directory_node::{parse_directory_node, DirEntry, DirectoryNode};
use crate::error::CoreError;
use crate::inode::{parse_inode_with_ceiling, ContentHandle, Inode};

/// Parsed metadata blob.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataBlob {
    pub inodes: Vec<Inode>,
    pub dir_nodes: Vec<DirectoryNode>,
}

impl MetadataBlob {
    /// True iff no inode or directory node records are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inodes.is_empty() && self.dir_nodes.is_empty()
    }

    /// Look up an inode by its `number`. Linear scan; metadata blobs
    /// are expected to be small enough that this is acceptable.
    #[must_use]
    pub fn inode_by_number(&self, number: u64) -> Option<&Inode> {
        self.inodes.iter().find(|i| i.number == number)
    }

    /// Look up a directory node by its BLAKE3 hash (the value carried
    /// in `ContentHandle::Directory`). Linear scan.
    #[must_use]
    pub fn dir_node_by_hash(&self, hash: &[u8; 32]) -> Option<&DirectoryNode> {
        self.dir_nodes
            .iter()
            .find(|n| dir_node_hash(&n.entries) == *hash)
    }

    /// Identify the root directory's inode number. The root is the
    /// unique directory inode whose `number` is not referenced by any
    /// other directory's entries — it has no parent.
    ///
    /// Returns `None` if the blob has no directory inodes, or if
    /// multiple inodes satisfy the criterion (indicating corruption or
    /// a multi-root layout we don't yet understand).
    #[must_use]
    pub fn root_inode_number(&self) -> Option<u64> {
        let mut referenced: std::collections::HashSet<u64> = std::collections::HashSet::new();
        for node in &self.dir_nodes {
            for entry in &node.entries {
                referenced.insert(entry.inode_number);
            }
        }
        let mut candidates = self.inodes.iter().filter_map(|i| {
            if i.is_directory() && !referenced.contains(&i.number) {
                Some(i.number)
            } else {
                None
            }
        });
        let first = candidates.next()?;
        if candidates.next().is_some() {
            None
        } else {
            Some(first)
        }
    }

    /// Build a path → inode-number index by walking the directory
    /// tree from the root.
    ///
    /// Returns a map from absolute POSIX-style paths (e.g. `"/a/b.txt"`)
    /// to inode numbers. Build cost is O(N) in the number of inodes;
    /// subsequent path lookups are O(1).
    ///
    /// Callers that resolve many paths against the same blob (e.g.
    /// `limni cat-multi`) should build this once rather than calling
    /// [`Self::inode_by_number`] + [`Self::dir_node_by_hash`] per
    /// component.
    #[must_use]
    pub fn build_path_index(&self) -> std::collections::HashMap<String, u64> {
        let mut index: std::collections::HashMap<String, u64> =
            std::collections::HashMap::with_capacity(self.inodes.len());
        let Some(root) = self.root_inode_number() else {
            return index;
        };
        index.insert(String::from("/"), root);
        // BFS from the root. Each directory's entries extend the
        // current path by one component.
        let mut queue: std::collections::VecDeque<(u64, String)> =
            std::collections::VecDeque::with_capacity(self.inodes.len());
        queue.push_back((root, String::from("/")));
        while let Some((inode_number, prefix)) = queue.pop_front() {
            let Some(inode) = self.inode_by_number(inode_number) else {
                continue;
            };
            let ContentHandle::Directory(dir_hash) = &inode.content_handle else {
                continue;
            };
            let Some(node) = self.dir_node_by_hash(dir_hash) else {
                continue;
            };
            for entry in &node.entries {
                let path = if prefix == "/" {
                    format!("/{}", entry.name)
                } else {
                    format!("{prefix}/{}", entry.name)
                };
                index.insert(path.clone(), entry.inode_number);
                queue.push_back((entry.inode_number, path));
            }
        }
        index
    }
}

/// Compute the BLAKE3 hash of a directory node's wire bytes — the same
/// hash a directory inode's `ContentHandle::Directory` carries.
///
/// This lives here (rather than in [`crate::directory_node`]) because
/// the writer's blob embeds pre-encoded bytes; the reader does not
/// necessarily re-hash from a fresh encoding. Keeping the algorithm
/// public lets callers re-derive the hash when needed.
///
/// # Panics
///
/// Panics if `entries.len()` or any name's UTF-8 byte length cannot fit
/// into a `u32`. Directory entries are validated upstream (their counts
/// and name lengths cannot exceed the directory node layout ceilings),
/// so this only fires on logic errors.
#[must_use]
pub fn dir_node_hash(entries: &[DirEntry]) -> [u8; 32] {
    let mut bytes = Vec::new();
    bytes.push(1u8); // version
    let count = u32::try_from(entries.len()).expect("entry count fits u32");
    bytes.extend_from_slice(&count.to_le_bytes());
    for entry in entries {
        let name_bytes = entry.name.as_bytes();
        let name_len = u32::try_from(name_bytes.len()).expect("name fits u32");
        bytes.extend_from_slice(&name_len.to_le_bytes());
        bytes.extend_from_slice(name_bytes);
        bytes.extend_from_slice(&entry.inode_number.to_le_bytes());
        bytes.push(entry.entry_type);
    }
    crate::merkle::hash_section(&bytes)
}

/// Parse the metadata blob from the cursor's current position.
///
/// Uses the default inline-data ceiling (4 KiB) for any inline-data
/// files carried inside inodes.
///
/// # Errors
///
/// - [`CoreError::Corrupt`] if the inode or directory node records
///   themselves fail to parse (delegated).
/// - [`CoreError::TooShort`] if the cursor underruns the count prefixes
///   or any inner record.
pub fn parse_metadata_blob(cursor: &mut ManifestCursor<'_>) -> Result<MetadataBlob, CoreError> {
    parse_metadata_blob_with_ceiling(cursor, crate::inode::DEFAULT_INLINE_DATA_MAX_BYTES)
}

/// Same as [`parse_metadata_blob`] but with a caller-supplied inline-data
/// ceiling.
///
/// # Errors
///
/// Inherits all errors from [`parse_metadata_blob`].
pub fn parse_metadata_blob_with_ceiling(
    cursor: &mut ManifestCursor<'_>,
    max_inline_bytes: u32,
) -> Result<MetadataBlob, CoreError> {
    let inode_count = cursor.read_u32_le()?;
    let inode_count_us = usize::try_from(inode_count).map_err(|_| CoreError::Corrupt {
        reason: format!("metadata blob inode_count {inode_count} exceeds usize"),
    })?;
    // DoS check: each inode needs at least INODE_FIXED_PREFIX_LEN + 1 (flags)
    // bytes before any optional fields. Reject before allocating.
    let min_inode_width = crate::inode::INODE_FIXED_PREFIX_LEN + 1;
    let min_inodes_size =
        inode_count_us
            .checked_mul(min_inode_width)
            .ok_or_else(|| CoreError::Corrupt {
                reason: format!("metadata blob inode_count {inode_count_us} overflows usize"),
            })?;
    if cursor.remaining_len() < min_inodes_size {
        return Err(CoreError::TooShort {
            have: cursor.remaining_len(),
            need: min_inodes_size,
        });
    }
    let mut inodes = Vec::with_capacity(inode_count_us);
    for _ in 0..inode_count_us {
        inodes.push(parse_inode_with_ceiling(cursor, max_inline_bytes)?);
    }

    let dir_node_count = cursor.read_u32_le()?;
    let dir_node_count_us = usize::try_from(dir_node_count).map_err(|_| CoreError::Corrupt {
        reason: format!("metadata blob dir_node_count {dir_node_count} exceeds usize"),
    })?;
    // Each directory node needs at least 1 (version) + 4 (entry_count) bytes
    // before any entries. Reject before allocating.
    let min_dir_node_width = 5;
    let min_dir_nodes_size = dir_node_count_us
        .checked_mul(min_dir_node_width)
        .ok_or_else(|| CoreError::Corrupt {
            reason: format!("metadata blob dir_node_count {dir_node_count_us} overflows usize"),
        })?;
    if cursor.remaining_len() < min_dir_nodes_size {
        return Err(CoreError::TooShort {
            have: cursor.remaining_len(),
            need: min_dir_nodes_size,
        });
    }
    let mut dir_nodes = Vec::with_capacity(dir_node_count_us);
    for _ in 0..dir_node_count_us {
        dir_nodes.push(parse_directory_node(cursor)?);
    }

    // Shared inline table: only present if bytes remain after dir_nodes.
    // Written by the writer when inline data dedup occurred.
    if cursor.remaining_len() >= 4 {
        let shared_count = cursor.read_u32_le()?;
        let shared_count_us = usize::try_from(shared_count).map_err(|_| CoreError::Corrupt {
            reason: format!("metadata blob shared_inline_count {shared_count} exceeds usize"),
        })?;
        let mut shared_table: Vec<Vec<u8>> = Vec::with_capacity(shared_count_us);
        for _ in 0..shared_count_us {
            let data_len = cursor.read_u32_le()?;
            let data_len_us = usize::try_from(data_len).map_err(|_| CoreError::Corrupt {
                reason: format!("shared inline entry len {data_len} exceeds usize"),
            })?;
            if data_len > max_inline_bytes {
                return Err(CoreError::Corrupt {
                    reason: format!(
                        "shared inline entry len {data_len} exceeds ceiling {max_inline_bytes}"
                    ),
                });
            }
            shared_table.push(cursor.read_n_owned(data_len_us)?);
        }
        // Resolve all SharedInline references to InlineData.
        for inode in &mut inodes {
            if let crate::inode::ContentHandle::SharedInline(idx) = &inode.content_handle {
                let data = shared_table.get(*idx).ok_or_else(|| CoreError::Corrupt {
                    reason: format!(
                        "shared inline index {idx} out of range (table has {} entries)",
                        shared_table.len()
                    ),
                })?;
                inode.content_handle = crate::inode::ContentHandle::InlineData(data.clone());
            }
        }
    }

    Ok(MetadataBlob { inodes, dir_nodes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inode::{ContentHandle, INODE_FLAG_INLINE_DATA, S_IFDIR, S_IFREG};

    fn make_regular_inline_inode(number: u64, mode: u32, data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&number.to_le_bytes());
        bytes.extend_from_slice(&mode.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.push(INODE_FLAG_INLINE_DATA);
        let len = u32::try_from(data.len()).expect("fits u32");
        bytes.extend_from_slice(&len.to_le_bytes());
        bytes.extend_from_slice(data);
        bytes
    }

    fn make_directory_inode(number: u64, hash: [u8; 32]) -> Vec<u8> {
        let mode = S_IFDIR | 0o755;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&number.to_le_bytes());
        bytes.extend_from_slice(&mode.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&hash);
        bytes
    }

    fn make_dir_node(entries: &[(&str, u64, u8)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(1u8);
        bytes.extend_from_slice(&u32::try_from(entries.len()).unwrap().to_le_bytes());
        for (name, inode_number, entry_type) in entries {
            let name_bytes = name.as_bytes();
            bytes.extend_from_slice(&u32::try_from(name_bytes.len()).unwrap().to_le_bytes());
            bytes.extend_from_slice(name_bytes);
            bytes.extend_from_slice(&inode_number.to_le_bytes());
            bytes.push(*entry_type);
        }
        bytes
    }

    fn make_blob(inode_bytes: &[Vec<u8>], dir_node_bytes: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&u32::try_from(inode_bytes.len()).unwrap().to_le_bytes());
        for inode in inode_bytes {
            bytes.extend_from_slice(inode);
        }
        bytes.extend_from_slice(&u32::try_from(dir_node_bytes.len()).unwrap().to_le_bytes());
        for node in dir_node_bytes {
            bytes.extend_from_slice(node);
        }
        bytes
    }

    #[test]
    fn parses_empty_metadata_blob() {
        let bytes = make_blob(&[], &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        let blob = parse_metadata_blob(&mut cursor).expect("empty blob parses");
        assert!(blob.is_empty());
        assert_eq!(cursor.position(), bytes.len());
    }

    #[test]
    fn parses_inline_inode() {
        let inode = make_regular_inline_inode(2, S_IFREG | 0o644, b"hello");
        let bytes = make_blob(&[inode], &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        let blob = parse_metadata_blob(&mut cursor).expect("blob parses");
        assert_eq!(blob.inodes.len(), 1);
        assert_eq!(blob.inodes[0].number, 2);
        assert!(blob.inodes[0].is_regular());
        match &blob.inodes[0].content_handle {
            ContentHandle::InlineData(d) => assert_eq!(d, b"hello"),
            other => panic!("expected InlineData, got {other:?}"),
        }
    }

    #[test]
    fn parses_directory_inode_with_node() {
        let node = make_dir_node(&[("a.txt", 2, 0x01), ("b.txt", 3, 0x01)]);
        let node_hash = crate::merkle::hash_section(&node);
        let inode = make_directory_inode(1, node_hash);
        let bytes = make_blob(&[inode], std::slice::from_ref(&node));
        let mut cursor = ManifestCursor::new(&bytes);
        let blob = parse_metadata_blob(&mut cursor).expect("blob parses");
        assert_eq!(blob.inodes.len(), 1);
        assert_eq!(blob.dir_nodes.len(), 1);
        assert!(blob.inodes[0].is_directory());
        assert_eq!(blob.dir_nodes[0].entries.len(), 2);
    }

    #[test]
    fn inode_lookup_finds_by_number() {
        let inode1 = make_regular_inline_inode(1, S_IFREG | 0o644, b"a");
        let inode2 = make_regular_inline_inode(2, S_IFREG | 0o644, b"b");
        let bytes = make_blob(&[inode1, inode2], &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        let blob = parse_metadata_blob(&mut cursor).expect("blob parses");
        assert!(blob.inode_by_number(1).is_some());
        assert!(blob.inode_by_number(2).is_some());
        assert!(blob.inode_by_number(99).is_none());
    }

    #[test]
    fn rejects_truncated_inode_count() {
        let bytes = [0u8; 2];
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_metadata_blob(&mut cursor) {
            Err(CoreError::TooShort { .. }) => {}
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn rejects_truncated_dir_node_count() {
        // 1 inode (regular inline) but truncated before the dir_node_count
        // u32.
        let inode = make_regular_inline_inode(1, S_IFREG | 0o644, b"x");
        let mut truncated = make_blob(&[inode], &[]);
        truncated.truncate(truncated.len() - 2);
        let mut cursor = ManifestCursor::new(&truncated);
        match parse_metadata_blob(&mut cursor) {
            Err(CoreError::TooShort { .. }) => {}
            other => panic!("expected TooShort, got {other:?}"),
        }
    }
}
