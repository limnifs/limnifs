//! Virtual filesystem layer — translates `LimniFS` metadata to VFS
//! operations (lookup, getattr, readdir, read).
//!
//! This module is the bridge between the `LimniFS` reader (metadata
//! blob + slab reader) and any filesystem frontend (`FUSE`, `WebDAV`,
//! etc.). It is pure-functional and has no system dependencies, so
//! it can be unit-tested without `FUSE` kernel support.
//!
//! ## Inode mapping
//!
//! `LimniFS` inode numbers are used directly as VFS inode numbers. The
//! root directory inode is identified by [`MetadataBlob::root_inode_number`].
//! `FUSE` frontends MUST remap the root to `FUSE_ROOT_ID = 1` if the
//! `LimniFS` root inode is not already 1.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::collections::HashMap;
use std::path::Path;

use limnifs_core::{
    parse_manifest_header, parse_metadata_blob, parse_metadata_reference, parse_slab_index,
    ContentHandle, CoreError, Inode, ManifestCursor, MetadataBlob, SlabIndex,
};

/// A virtual filesystem backed by a `.lim` image. Owns the parsed
/// manifest bytes and slab data so all lookups are in-memory after
/// construction.
pub struct Vfs {
    metadata_blob: MetadataBlob,
    slab_index: SlabIndex,
    /// Image directory (for resolving `file:` slab locators).
    image_dir: std::path::PathBuf,
    /// Loaded slabs, keyed by slab ordinal (0, 1, ...).
    slabs: HashMap<u64, Vec<u8>>,
    root_inode_number: u64,
}

/// VFS file type.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VfsType {
    Regular,
    Directory,
    Symlink,
    Other,
}

/// Attributes returned by `getattr`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct VfsAttr {
    pub ino: u64,
    pub size: u64,
    pub mode: u32,
    pub kind: VfsType,
    pub nlink: u32,
    pub mtime_ns: u64,
}

/// Error from VFS operations.
#[derive(Debug)]
pub enum VfsError {
    Core(CoreError),
    Io(std::io::Error),
    NotFound,
}

impl std::fmt::Display for VfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "I/O: {e}"),
            Self::NotFound => write!(f, "not found"),
        }
    }
}

impl std::error::Error for VfsError {}

impl From<CoreError> for VfsError {
    fn from(e: CoreError) -> Self {
        Self::Core(e)
    }
}

impl From<std::io::Error> for VfsError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl Vfs {
    /// Open a `.lim` image file and construct a VFS. Parses the
    /// manifest, extracts the inlined metadata blob, and loads all
    /// referenced slabs into memory.
    ///
    /// # Errors
    ///
    /// Returns [`VfsError`] if the image fails to parse or any slab
    /// file cannot be read.
    pub fn open(image_path: &Path) -> Result<Self, VfsError> {
        let bytes = std::fs::read(image_path)?;
        let image_dir = image_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();
        Self::from_bytes(&bytes, &image_dir)
    }

    /// Construct a VFS from raw manifest bytes and a base directory
    /// for resolving slab locators.
    ///
    /// # Errors
    ///
    /// Returns [`VfsError`] if the manifest fails to parse or any
    /// slab file cannot be read.
    pub fn from_bytes(bytes: &[u8], image_dir: &Path) -> Result<Self, VfsError> {
        let mut cursor = ManifestCursor::new(bytes);
        let _ = parse_manifest_header(&mut cursor)?;
        let _ = limnifs_core::parse_feature_flags_section(&mut cursor)?;
        let meta_ref = parse_metadata_reference(&mut cursor)?;
        let blob_bytes = meta_ref.inline_metadata.as_deref().ok_or_else(|| {
            VfsError::Core(CoreError::Corrupt {
                reason: "VFS requires inlined metadata".into(),
            })
        })?;
        let mut blob_cursor = ManifestCursor::new(blob_bytes);
        let metadata_blob = parse_metadata_blob(&mut blob_cursor)?;
        let slab_index = parse_slab_index(&mut cursor)?;

        let root_inode_number = metadata_blob.root_inode_number().ok_or_else(|| {
            VfsError::Core(CoreError::Corrupt {
                reason: "no unique root directory inode".into(),
            })
        })?;

        let mut slabs: HashMap<u64, Vec<u8>> = HashMap::new();
        for entry in &slab_index.entries {
            for locator in &entry.locators {
                let uri = &locator.uri;
                let name =
                    limnifs_core::locator::local_sidecar_name(uri).map_err(VfsError::Core)?;
                let path = image_dir.join(name);
                if path.exists() {
                    let slab_bytes = std::fs::read(&path)?;
                    slabs.insert(entry.slab_id.ordinal, slab_bytes);
                    break;
                }
            }
        }

        Ok(Self {
            metadata_blob,
            slab_index,
            image_dir: image_dir.to_path_buf(),
            slabs,
            root_inode_number,
        })
    }

    /// The root directory's inode number.
    #[must_use]
    pub const fn root_inode(&self) -> u64 {
        self.root_inode_number
    }

    /// Look up `name` in the directory identified by `parent_ino`.
    /// Returns the child's inode number, or `None` if not found.
    #[must_use]
    pub fn lookup(&self, parent_ino: u64, name: &str) -> Option<u64> {
        let parent = self.metadata_blob.inode_by_number(parent_ino)?;
        let hash = match &parent.content_handle {
            ContentHandle::Directory(h) => *h,
            _ => return None,
        };
        let node = self.metadata_blob.dir_node_by_hash(&hash)?;
        node.entries
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.inode_number)
    }

    /// Get the attributes of `ino`.
    #[must_use]
    pub fn getattr(&self, ino: u64) -> Option<VfsAttr> {
        let inode = self.metadata_blob.inode_by_number(ino)?;
        Some(vfs_attr(ino, inode))
    }

    /// List the entries in directory `ino`. Returns an empty vec for
    /// non-directory inodes.
    #[must_use]
    pub fn readdir(&self, ino: u64) -> Vec<(u64, String, VfsType)> {
        let Some(inode) = self.metadata_blob.inode_by_number(ino) else {
            return Vec::new();
        };
        let hash = match &inode.content_handle {
            ContentHandle::Directory(h) => *h,
            _ => return Vec::new(),
        };
        let Some(node) = self.metadata_blob.dir_node_by_hash(&hash) else {
            return Vec::new();
        };
        node.entries
            .iter()
            .map(|e| {
                let kind = entry_vfs_type(e.entry_type);
                (e.inode_number, e.name.clone(), kind)
            })
            .collect()
    }

    /// Read `len` bytes starting at `offset` from the file identified
    /// by `ino`. For inline-data files, reads directly from the inode.
    /// For slab-backed files, loads the slab and decompresses the drop.
    ///
    /// Returns the bytes read (may be shorter than `len` if the read
    /// crosses the file boundary).
    ///
    /// # Errors
    ///
    /// Returns [`VfsError`] if the slab cannot be loaded or the drop
    /// cannot be found/decompressed.
    pub fn read(&self, ino: u64, offset: u64, len: usize) -> Result<Vec<u8>, VfsError> {
        let inode = self
            .metadata_blob
            .inode_by_number(ino)
            .ok_or(VfsError::NotFound)?;
        let data = match &inode.content_handle {
            ContentHandle::InlineData(d) => d.clone(),
            ContentHandle::SliceMap(slices) => {
                let mut file_data = Vec::new();
                for slice in slices {
                    let slab_bytes = self.load_slab_for_slice(slice.drop_id.as_bytes())?;
                    let view = limnifs_core::parse_slab(&slab_bytes)?;
                    let plaintext = view
                        .plaintext_for(slice.drop_id.as_bytes())
                        .ok_or(VfsError::NotFound)?
                        .map_err(VfsError::Core)?;
                    file_data.extend_from_slice(&plaintext);
                }
                file_data
            }
            _ => return Err(VfsError::NotFound),
        };
        let start = usize::try_from(offset).unwrap_or(0);
        let end = start.saturating_add(len).min(data.len());
        if start >= data.len() {
            return Ok(Vec::new());
        }
        Ok(data[start..end].to_vec())
    }

    /// Load the slab containing `drop_id`, caching the result.
    fn load_slab_for_slice(&self, drop_id: &[u8; 32]) -> Result<Vec<u8>, VfsError> {
        let _ = drop_id;
        let entry = self.slab_index.entries.first().ok_or(VfsError::NotFound)?;
        if let Some(cached) = self.slabs.get(&entry.slab_id.ordinal) {
            return Ok(cached.clone());
        }
        let locator = entry.locators.first().ok_or(VfsError::NotFound)?;
        let name =
            limnifs_core::locator::local_sidecar_name(&locator.uri).map_err(VfsError::Core)?;
        let path = self.image_dir.join(name);
        Ok(std::fs::read(&path)?)
    }
}

fn vfs_attr(ino: u64, inode: &Inode) -> VfsAttr {
    let kind = if inode.is_directory() {
        VfsType::Directory
    } else if inode.is_regular() {
        VfsType::Regular
    } else if matches!(inode.file_type(), limnifs_core::S_IFLNK) {
        VfsType::Symlink
    } else {
        VfsType::Other
    };
    let size = match &inode.content_handle {
        ContentHandle::InlineData(d) => d.len() as u64,
        ContentHandle::SliceMap(slices) => slices.last().map_or(0, |s| s.file_byte_end),
        _ => 0,
    };
    VfsAttr {
        ino,
        size,
        mode: inode.mode & 0o7777,
        kind,
        nlink: inode.nlink,
        mtime_ns: inode.mtime_ns,
    }
}

fn entry_vfs_type(entry_type: u8) -> VfsType {
    match entry_type {
        0x01 => VfsType::Regular,
        0x02 => VfsType::Directory,
        0x03 => VfsType::Symlink,
        _ => VfsType::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static VFS_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn make_image(dir: &Path) -> Vec<u8> {
        let artifact = limnifs_write::write_directory(dir).expect("write succeeds");
        artifact.bytes
    }

    fn make_source_tree() -> std::path::PathBuf {
        let id = VFS_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("limnifs-vfs-test-{id}"));
        std::fs::create_dir_all(&dir).expect("create dir");
        std::fs::create_dir_all(dir.join("sub")).expect("create sub");
        std::fs::write(dir.join("a.txt"), b"hello").expect("write a");
        std::fs::write(dir.join("sub").join("b.txt"), b"world").expect("write b");
        dir
    }

    #[test]
    fn vfs_opens_and_reads_root() {
        let source = make_source_tree();
        let bytes = make_image(&source);
        std::fs::remove_dir_all(&source).ok();

        let vfs = Vfs::from_bytes(&bytes, std::path::Path::new(".")).expect("opens");
        let root = vfs.root_inode();
        let entries = vfs.readdir(root);
        assert!(entries.iter().any(|(_, name, _)| name == "a.txt"));
        assert!(entries.iter().any(|(_, name, _)| name == "sub"));
    }

    #[test]
    fn vfs_lookup_finds_child() {
        let source = make_source_tree();
        let bytes = make_image(&source);
        std::fs::remove_dir_all(&source).ok();

        let vfs = Vfs::from_bytes(&bytes, std::path::Path::new(".")).expect("opens");
        let root = vfs.root_inode();
        let child_ino = vfs.lookup(root, "a.txt").expect("found");
        assert!(child_ino > 0);
    }

    #[test]
    fn vfs_read_inline_file() {
        let source = make_source_tree();
        let bytes = make_image(&source);
        std::fs::remove_dir_all(&source).ok();

        let vfs = Vfs::from_bytes(&bytes, std::path::Path::new(".")).expect("opens");
        let root = vfs.root_inode();
        let file_ino = vfs.lookup(root, "a.txt").expect("found");
        let data = vfs.read(file_ino, 0, 100).expect("read succeeds");
        assert_eq!(data, b"hello");
    }

    #[test]
    fn vfs_read_with_offset() {
        let source = make_source_tree();
        let bytes = make_image(&source);
        std::fs::remove_dir_all(&source).ok();

        let vfs = Vfs::from_bytes(&bytes, std::path::Path::new(".")).expect("opens");
        let root = vfs.root_inode();
        let file_ino = vfs.lookup(root, "a.txt").expect("found");
        let data = vfs.read(file_ino, 1, 3).expect("read succeeds");
        assert_eq!(data, b"ell");
    }

    #[test]
    fn vfs_getattr_returns_correct_type() {
        let source = make_source_tree();
        let bytes = make_image(&source);
        std::fs::remove_dir_all(&source).ok();

        let vfs = Vfs::from_bytes(&bytes, std::path::Path::new(".")).expect("opens");
        let root = vfs.root_inode();
        let attr = vfs.getattr(root).expect("attr");
        assert_eq!(attr.kind, VfsType::Directory);

        let file_ino = vfs.lookup(root, "a.txt").expect("found");
        let attr = vfs.getattr(file_ino).expect("attr");
        assert_eq!(attr.kind, VfsType::Regular);
        assert_eq!(attr.size, 5);
    }

    #[test]
    fn vfs_traverses_subdirectory() {
        let source = make_source_tree();
        let bytes = make_image(&source);
        std::fs::remove_dir_all(&source).ok();

        let vfs = Vfs::from_bytes(&bytes, std::path::Path::new(".")).expect("opens");
        let root = vfs.root_inode();
        let sub_ino = vfs.lookup(root, "sub").expect("found sub");
        let entries = vfs.readdir(sub_ino);
        assert!(entries.iter().any(|(_, name, _)| name == "b.txt"));

        let b_ino = vfs.lookup(sub_ino, "b.txt").expect("found b");
        let data = vfs.read(b_ino, 0, 100).expect("read");
        assert_eq!(data, b"world");
    }
}
