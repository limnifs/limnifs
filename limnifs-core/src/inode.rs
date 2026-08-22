//! Inode record (spec §4.1, `bit-level/33-inode.md`).
//!
//! An inode represents one filesystem object. Every entry in the
//! directory tree references an inode by its `number`. The inode
//! carries POSIX attributes, optional xattrs, and a type-dependent
//! content handle.

use crate::cursor::ManifestCursor;
use crate::error::CoreError;
use limnifs_format::DropId;

/// Width of the fixed prefix (before optional atime / xattrs / content handle).
pub const INODE_FIXED_PREFIX_LEN: usize = 41;

/// Flag: `atime_ns` field is present.
pub const INODE_FLAG_ATIME: u8 = 0x01;
/// Flag: xattr block is present.
pub const INODE_FLAG_HAS_XATTRS: u8 = 0x02;
/// Flag: inline data is present (regular files only).
pub const INODE_FLAG_INLINE_DATA: u8 = 0x04;
/// Flag bit indicating the inode's inline data is a shared-table
/// reference (deduplicated). When set alongside `INODE_FLAG_INLINE_DATA`,
/// the content handle body is a u32 index into the shared inline
/// table at the end of the metadata blob, not inline bytes.
pub const INODE_FLAG_SHARED_INLINE: u8 = 0x08;
/// Mask for reserved flag bits (4-7). Bit 3 is the DEFINED
/// [`INODE_FLAG_SHARED_INLINE`]; the previous value (0xF8) covered
/// it, making the reader reject every deduplicated shared-inline
/// inode the writer emits (issue #186).
pub const INODE_FLAG_RESERVED_MASK: u8 = 0xF0;

/// POSIX file type bits from `mode`.
pub const S_IFMT: u32 = 0xF000;
pub const S_IFREG: u32 = 0x8000;
pub const S_IFDIR: u32 = 0x4000;
pub const S_IFLNK: u32 = 0xA000;
pub const S_IFBLK: u32 = 0x6000;
pub const S_IFCHR: u32 = 0x2000;
pub const S_IFIFO: u32 = 0x1000;
pub const S_IFSOCK: u32 = 0xC000;

/// Default inline-data ceiling (spec §4.3: threshold is spec-pinned).
pub const DEFAULT_INLINE_DATA_MAX_BYTES: u32 = 4 * 1024;

/// One extended attribute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XAttr {
    pub namespace: u8,
    pub key: String,
    pub value: Vec<u8>,
}

/// Type-dependent content handle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContentHandle {
    /// Regular file with inline data.
    InlineData(Vec<u8>),
    /// Regular file whose inline data is in the shared inline table
    /// (deduplicated). The index is resolved to `InlineData` after
    /// the full metadata blob is parsed.
    SharedInline(usize),
    /// Regular file with a slice map.
    SliceMap(Vec<SliceRef>),
    /// Directory: BLAKE3 hash of the root Merkle B-tree node.
    Directory([u8; 32]),
    /// Symlink: target path.
    Symlink(String),
    /// Block or char device: device number.
    Device(u64),
    /// FIFO or socket: pipe identifier.
    Pipe(u64),
}

/// One entry in a slice map: maps a byte range of the file to a
/// byte range of a drop's plaintext.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SliceRef {
    pub file_byte_start: u64,
    pub file_byte_end: u64,
    pub drop_id: DropId,
    pub drop_byte_start: u32,
    pub drop_byte_len: u32,
}

/// Parsed inode record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Inode {
    pub number: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub mtime_ns: u64,
    pub ctime_ns: u64,
    pub nlink: u32,
    pub atime_ns: Option<u64>,
    pub xattrs: Vec<XAttr>,
    pub content_handle: ContentHandle,
}

impl Inode {
    /// Returns the file type bits from `mode`.
    #[must_use]
    pub fn file_type(&self) -> u32 {
        self.mode & S_IFMT
    }

    /// True iff this inode is a regular file.
    #[must_use]
    pub fn is_regular(&self) -> bool {
        self.file_type() == S_IFREG
    }

    /// True iff this inode is a directory.
    #[must_use]
    pub fn is_directory(&self) -> bool {
        self.file_type() == S_IFDIR
    }
}

/// Parse an inode record from the cursor's current position.
///
/// Uses the default inline-data ceiling (4 KiB).
///
/// # Errors
///
/// - [`CoreError::Corrupt`] if reserved flag bits are set, the file
///   type is unknown, the inline data exceeds the ceiling, or any
///   slice/xattr field is malformed.
/// - [`CoreError::TooShort`] if the cursor underruns.
pub fn parse_inode(cursor: &mut ManifestCursor<'_>) -> Result<Inode, CoreError> {
    parse_inode_with_ceiling(cursor, DEFAULT_INLINE_DATA_MAX_BYTES)
}

/// Same as [`parse_inode`] but with a caller-supplied inline-data ceiling.
///
/// # Errors
///
/// Inherits all errors from [`parse_inode`].
pub fn parse_inode_with_ceiling(
    cursor: &mut ManifestCursor<'_>,
    max_inline_bytes: u32,
) -> Result<Inode, CoreError> {
    let number = cursor.read_u64_le()?;
    let mode = cursor.read_u32_le()?;
    let uid = cursor.read_u32_le()?;
    let gid = cursor.read_u32_le()?;
    let mtime_ns = cursor.read_u64_le()?;
    let ctime_ns = cursor.read_u64_le()?;
    let nlink = cursor.read_u32_le()?;
    let flags = cursor.read_u8()?;

    if flags & INODE_FLAG_RESERVED_MASK != 0 {
        return Err(CoreError::Corrupt {
            reason: format!("inode {number}: reserved flag bits set (0x{flags:02X})"),
        });
    }

    let atime_ns = if flags & INODE_FLAG_ATIME != 0 {
        Some(cursor.read_u64_le()?)
    } else {
        None
    };

    let xattrs = if flags & INODE_FLAG_HAS_XATTRS != 0 {
        parse_xattr_block(cursor)?
    } else {
        Vec::new()
    };

    let content_handle = parse_content_handle(cursor, mode, flags, number, max_inline_bytes)?;

    Ok(Inode {
        number,
        mode,
        uid,
        gid,
        mtime_ns,
        ctime_ns,
        nlink,
        atime_ns,
        xattrs,
        content_handle,
    })
}

fn parse_xattr_block(cursor: &mut ManifestCursor<'_>) -> Result<Vec<XAttr>, CoreError> {
    let count = cursor.read_u32_le()?;
    let count_us = usize::try_from(count).map_err(|_| CoreError::Corrupt {
        reason: format!("xattr_count {count} exceeds usize"),
    })?;
    let mut xattrs = Vec::with_capacity(count_us);
    for _ in 0..count_us {
        let namespace = cursor.read_u8()?;
        if namespace > 0x03 {
            return Err(CoreError::Corrupt {
                reason: format!("xattr namespace 0x{namespace:02X} out of range (0x00..0x03)"),
            });
        }
        let key_len = cursor.read_u32_le()?;
        let key_len_us = usize::try_from(key_len).map_err(|_| CoreError::Corrupt {
            reason: format!("xattr key_len {key_len} exceeds usize"),
        })?;
        let key_bytes = cursor.read_n(key_len_us)?;
        let key = std::str::from_utf8(key_bytes).map_err(|_| CoreError::Corrupt {
            reason: "xattr key is not valid UTF-8".into(),
        })?;
        if key.contains('\0') {
            return Err(CoreError::Corrupt {
                reason: "xattr key contains NUL byte".into(),
            });
        }
        let value_len = cursor.read_u32_le()?;
        let value_len_us = usize::try_from(value_len).map_err(|_| CoreError::Corrupt {
            reason: format!("xattr value_len {value_len} exceeds usize"),
        })?;
        let value = cursor.read_n_owned(value_len_us)?;
        xattrs.push(XAttr {
            namespace,
            key: key.to_owned(),
            value,
        });
    }
    Ok(xattrs)
}

fn parse_content_handle(
    cursor: &mut ManifestCursor<'_>,
    mode: u32,
    flags: u8,
    inode_number: u64,
    max_inline_bytes: u32,
) -> Result<ContentHandle, CoreError> {
    let file_type = mode & S_IFMT;
    match file_type {
        S_IFREG => {
            if flags & INODE_FLAG_SHARED_INLINE != 0 {
                // Shared inline: content is a u32 index into the
                // shared inline table at the end of the metadata blob.
                // The caller (parse_metadata_blob) resolves it.
                let index = cursor.read_u32_le()?;
                let index_us = usize::try_from(index).map_err(|_| CoreError::Corrupt {
                    reason: format!("shared_inline_index {index} exceeds usize"),
                })?;
                Ok(ContentHandle::SharedInline(index_us))
            } else if flags & INODE_FLAG_INLINE_DATA != 0 {
                let inline_len = cursor.read_u32_le()?;
                if inline_len > max_inline_bytes {
                    return Err(CoreError::Corrupt {
                        reason: format!(
                            "inode {inode_number}: inline_data_len {inline_len} exceeds ceiling {max_inline_bytes}"
                        ),
                    });
                }
                let inline_len_us =
                    usize::try_from(inline_len).map_err(|_| CoreError::Corrupt {
                        reason: format!("inline_data_len {inline_len} exceeds usize"),
                    })?;
                let data = cursor.read_n_owned(inline_len_us)?;
                Ok(ContentHandle::InlineData(data))
            } else {
                let slice_count = cursor.read_u32_le()?;
                let count_us = usize::try_from(slice_count).map_err(|_| CoreError::Corrupt {
                    reason: format!("slice_count {slice_count} exceeds usize"),
                })?;
                let mut slices = Vec::with_capacity(count_us);
                for _ in 0..count_us {
                    let file_byte_start = cursor.read_u64_le()?;
                    let file_byte_end = cursor.read_u64_le()?;
                    if file_byte_start >= file_byte_end {
                        return Err(CoreError::Corrupt {
                            reason: format!(
                                "inode {inode_number}: slice has file_byte_start ({file_byte_start}) >= file_byte_end ({file_byte_end})"
                            ),
                        });
                    }
                    let drop_id_bytes = cursor.read_n(32)?;
                    let mut drop_id_arr = [0u8; 32];
                    drop_id_arr.copy_from_slice(drop_id_bytes);
                    let drop_id = DropId::from_bytes(drop_id_arr);
                    let drop_byte_start = cursor.read_u32_le()?;
                    let drop_byte_len = cursor.read_u32_le()?;
                    slices.push(SliceRef {
                        file_byte_start,
                        file_byte_end,
                        drop_id,
                        drop_byte_start,
                        drop_byte_len,
                    });
                }
                Ok(ContentHandle::SliceMap(slices))
            }
        }
        S_IFDIR => {
            let hash_bytes = cursor.read_n(32)?;
            let mut hash = [0u8; 32];
            hash.copy_from_slice(hash_bytes);
            Ok(ContentHandle::Directory(hash))
        }
        S_IFLNK => {
            let target_len = cursor.read_u32_le()?;
            let target_len_us = usize::try_from(target_len).map_err(|_| CoreError::Corrupt {
                reason: format!("target_len {target_len} exceeds usize"),
            })?;
            let target_bytes = cursor.read_n(target_len_us)?;
            let target = std::str::from_utf8(target_bytes).map_err(|_| CoreError::Corrupt {
                reason: "symlink target is not valid UTF-8".into(),
            })?;
            Ok(ContentHandle::Symlink(target.to_owned()))
        }
        S_IFBLK | S_IFCHR => {
            let dev = cursor.read_u64_le()?;
            Ok(ContentHandle::Device(dev))
        }
        S_IFIFO | S_IFSOCK => {
            let pipe_id = cursor.read_u64_le()?;
            Ok(ContentHandle::Pipe(pipe_id))
        }
        _ => Err(CoreError::Corrupt {
            reason: format!("inode {inode_number}: unknown file type 0x{file_type:04X}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_regular_inline_inode(number: u64, mode: u32, inline_data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&number.to_le_bytes());
        bytes.extend_from_slice(&mode.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // uid
        bytes.extend_from_slice(&0u32.to_le_bytes()); // gid
        bytes.extend_from_slice(&0u64.to_le_bytes()); // mtime
        bytes.extend_from_slice(&0u64.to_le_bytes()); // ctime
        bytes.extend_from_slice(&1u32.to_le_bytes()); // nlink
        bytes.push(INODE_FLAG_INLINE_DATA); // flags
        let inline_len = u32::try_from(inline_data.len()).unwrap();
        bytes.extend_from_slice(&inline_len.to_le_bytes());
        bytes.extend_from_slice(inline_data);
        bytes
    }

    fn make_directory_inode(number: u64) -> Vec<u8> {
        let mode = S_IFDIR | 0o755;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&number.to_le_bytes());
        bytes.extend_from_slice(&mode.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // uid
        bytes.extend_from_slice(&0u32.to_le_bytes()); // gid
        bytes.extend_from_slice(&0u64.to_le_bytes()); // mtime
        bytes.extend_from_slice(&0u64.to_le_bytes()); // ctime
        bytes.extend_from_slice(&2u32.to_le_bytes()); // nlink
        bytes.push(0); // flags
        bytes.extend_from_slice(&[0xBB; 32]); // btree_node_hash
        bytes
    }

    #[test]
    fn parses_regular_inline_file() {
        let data = b"hello world";
        let bytes = make_regular_inline_inode(42, S_IFREG | 0o644, data);
        let mut cursor = ManifestCursor::new(&bytes);
        let inode = parse_inode(&mut cursor).expect("inline file parses");
        assert_eq!(inode.number, 42);
        assert!(inode.is_regular());
        assert!(!inode.is_directory());
        assert!(inode.atime_ns.is_none());
        assert!(inode.xattrs.is_empty());
        match &inode.content_handle {
            ContentHandle::InlineData(d) => assert_eq!(d, data),
            other => panic!("expected InlineData, got {other:?}"),
        }
        assert_eq!(cursor.position(), bytes.len());
    }

    #[test]
    fn parses_directory() {
        let bytes = make_directory_inode(0);
        let mut cursor = ManifestCursor::new(&bytes);
        let inode = parse_inode(&mut cursor).expect("directory parses");
        assert_eq!(inode.number, 0);
        assert!(inode.is_directory());
        match &inode.content_handle {
            ContentHandle::Directory(hash) => assert_eq!(hash, &[0xBB; 32]),
            other => panic!("expected Directory, got {other:?}"),
        }
    }

    #[test]
    fn rejects_reserved_flag_bits() {
        let mut bytes = make_regular_inline_inode(1, S_IFREG | 0o644, b"x");
        // Set a reserved flag bit (bit 4; bit 3 is the defined
        // SHARED_INLINE flag — see issue #186).
        bytes[INODE_FIXED_PREFIX_LEN - 1] |= 0x10;
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_inode(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("reserved"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_file_type() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&0x0000u32.to_le_bytes()); // mode = unknown type
        bytes.extend_from_slice(&0u32.to_le_bytes()); // uid
        bytes.extend_from_slice(&0u32.to_le_bytes()); // gid
        bytes.extend_from_slice(&0u64.to_le_bytes()); // mtime
        bytes.extend_from_slice(&0u64.to_le_bytes()); // ctime
        bytes.extend_from_slice(&1u32.to_le_bytes()); // nlink
        bytes.push(0); // flags
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_inode(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("unknown file type"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn parses_with_atime() {
        let mode = S_IFREG | 0o644;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&mode.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // uid
        bytes.extend_from_slice(&0u32.to_le_bytes()); // gid
        bytes.extend_from_slice(&0u64.to_le_bytes()); // mtime
        bytes.extend_from_slice(&0u64.to_le_bytes()); // ctime
        bytes.extend_from_slice(&1u32.to_le_bytes()); // nlink
        bytes.push(INODE_FLAG_ATIME); // flags = atime only
        bytes.extend_from_slice(&999u64.to_le_bytes()); // atime_ns
        bytes.extend_from_slice(&0u32.to_le_bytes()); // slice_count = 0 (empty slice map)
        let mut cursor = ManifestCursor::new(&bytes);
        let inode = parse_inode(&mut cursor).expect("atime inode parses");
        assert_eq!(inode.atime_ns, Some(999));
    }

    #[test]
    fn rejects_inline_above_ceiling() {
        let oversized = vec![0xFF; (DEFAULT_INLINE_DATA_MAX_BYTES as usize) + 1];
        let bytes = make_regular_inline_inode(1, S_IFREG | 0o644, &oversized);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_inode(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("ceiling"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }
}
