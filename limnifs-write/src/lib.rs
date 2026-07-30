//! `LimniFS` writer pipeline — directory tree to `.lim` image.
//!
//! The minimum viable writer takes a real directory tree and produces
//! a valid `.lim` manifest artifact with inlined metadata. All files
//! under the inline threshold (4 KiB) are stored as inline data in
//! their inodes; larger files are currently unsupported (future
//! versions will use `FastCDC` + slab packing per §6).
//!
//! ## Usage
//!
//! ```no_run
//! use limnifs_write::write_directory;
//! let artifact = write_directory("/path/to/dir")?;
//! std::fs::write("output.lim", &artifact.bytes)?;
//! ```

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::path::Path;

use limnifs_core::{
    compute_merkle_root, hash_empty_section, hash_section, ManifestHeader, SectionHashes,
    FEATURE_FLAGS_SECTION_VERSION, HISTORY_SECTION_VERSION, METADATA_REFERENCE_SECTION_VERSION,
    SLAB_INDEX_SECTION_VERSION,
};
use limnifs_format::ManifestRoot;

/// Inline-data threshold: files at or below this size get inline data
/// in their inode. Larger files are rejected by the MVP writer (will
/// be supported when slab packing lands).
pub const INLINE_THRESHOLD: usize = 4096;

/// Result of writing a directory tree.
#[derive(Clone, Debug)]
pub struct WriteArtifact {
    pub bytes: Vec<u8>,
    pub merkle_root: ManifestRoot,
    pub inode_count: usize,
    pub file_count: usize,
    pub dir_count: usize,
}

/// Error during writing.
#[derive(Debug)]
pub enum WriteError {
    Io(std::io::Error),
    FileTooLarge { path: String, size: u64 },
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::FileTooLarge { path, size } => write!(
                f,
                "file {path} is {size} bytes (MVP writer only supports files <= {INLINE_THRESHOLD} bytes)"
            ),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Walk a directory tree and produce a valid `.lim` manifest artifact
/// with inlined metadata. All files must be at or below
/// [`INLINE_THRESHOLD`] bytes.
///
/// # Errors
///
/// Returns [`WriteError::Io`] for filesystem errors or
/// [`WriteError::FileTooLarge`] for files exceeding the inline threshold.
pub fn write_directory(root: &Path) -> Result<WriteArtifact, WriteError> {
    let mut ctx = WriteContext::new();
    let root_inode_number = ctx.walk(root)?;
    ctx.root_inode_number = root_inode_number;
    let artifact = ctx.assemble();
    Ok(artifact)
}

struct PendingInode {
    number: u64,
    mode: u32,
    mtime_ns: u64,
    content: PendingContent,
}

enum PendingContent {
    Inline(Vec<u8>),
    Directory(Vec<(String, u64, u8)>),
}

struct DirNode {
    entries: Vec<(String, u64, u8)>,
    bytes: Vec<u8>,
    hash: [u8; 32],
}

struct WriteContext {
    next_inode: u64,
    inodes: Vec<PendingInode>,
    dir_nodes: Vec<DirNode>,
    file_count: usize,
    dir_count: usize,
    root_inode_number: u64,
}

impl WriteContext {
    fn new() -> Self {
        Self {
            next_inode: 1,
            inodes: Vec::new(),
            dir_nodes: Vec::new(),
            file_count: 0,
            dir_count: 0,
            root_inode_number: 0,
        }
    }

    fn alloc_inode(&mut self) -> u64 {
        let n = self.next_inode;
        self.next_inode += 1;
        n
    }

    fn walk(&mut self, path: &Path) -> Result<u64, WriteError> {
        let meta = std::fs::symlink_metadata(path)?;
        let file_type = meta.file_type();
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0u128, |d| d.as_nanos());
        let mtime_ns: u64 = mtime_ns.try_into().unwrap_or(0);

        if file_type.is_dir() {
            self.dir_count += 1;
            let inode_number = self.alloc_inode();
            let mut entries: Vec<(String, u64, u8)> = Vec::new();

            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let child_path = entry.path();
                let child_inode = self.walk(&child_path)?;
                let child_meta = entry.metadata()?;
                let entry_type = if child_meta.is_dir() { 0x02 } else { 0x01 };
                entries.push((name, child_inode, entry_type));
            }

            entries.sort_by(|a, b| a.0.cmp(&b.0));

            let dir_node = encode_dir_node(&entries);
            self.dir_nodes.push(dir_node);

            self.inodes.push(PendingInode {
                number: inode_number,
                mode: 0o040_755,
                mtime_ns,
                content: PendingContent::Directory(entries),
            });

            Ok(inode_number)
        } else if file_type.is_file() {
            self.file_count += 1;
            let inode_number = self.alloc_inode();
            let size = meta.len();
            if usize::try_from(size).map_or(true, |s| s > INLINE_THRESHOLD) {
                return Err(WriteError::FileTooLarge {
                    path: path.display().to_string(),
                    size,
                });
            }
            let data = std::fs::read(path)?;
            self.inodes.push(PendingInode {
                number: inode_number,
                mode: 0o100_644,
                mtime_ns,
                content: PendingContent::Inline(data),
            });
            Ok(inode_number)
        } else {
            Err(WriteError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("unsupported file type: {}", path.display()),
            )))
        }
    }

    fn assemble(self) -> WriteArtifact {
        let inode_count = self.inodes.len();
        let dir_count = self.dir_count;

        // Build the metadata blob: [inode_count][inodes][dir_node_count][dir_nodes].
        let mut metadata_blob = Vec::new();
        metadata_blob.extend_from_slice(&u32::try_from(self.inodes.len()).unwrap().to_le_bytes());
        for inode in &self.inodes {
            self.encode_inode(&mut metadata_blob, inode);
        }
        metadata_blob
            .extend_from_slice(&u32::try_from(self.dir_nodes.len()).unwrap().to_le_bytes());
        for node in &self.dir_nodes {
            metadata_blob.extend_from_slice(&node.bytes);
        }

        // Assemble the manifest sections in spec order.
        let mut manifest = Vec::new();

        // §5.1 Header
        let header_start = manifest.len();
        let header = ManifestHeader::current();
        manifest.extend_from_slice(&header.to_bytes());
        let header_end = manifest.len();

        // §5.2 Feature flags (empty)
        let flags_start = manifest.len();
        manifest.push(FEATURE_FLAGS_SECTION_VERSION);
        manifest.extend_from_slice(&0u32.to_le_bytes());
        let flags_end = manifest.len();

        // §5.3 Metadata reference (inlined)
        let meta_ref_start = manifest.len();
        manifest.push(METADATA_REFERENCE_SECTION_VERSION);
        let metadata_hash = hash_section(&metadata_blob);
        manifest.extend_from_slice(&metadata_hash);
        manifest.extend_from_slice(&0u32.to_le_bytes()); // locator_count = 0
        let inline_len = u32::try_from(metadata_blob.len()).expect("metadata fits u32");
        manifest.extend_from_slice(&inline_len.to_le_bytes());
        manifest.extend_from_slice(&metadata_blob);
        let meta_ref_end = manifest.len();

        // §5.4 Slab index (empty — no drops/slabs for inline-only images)
        let slab_index_start = manifest.len();
        manifest.push(SLAB_INDEX_SECTION_VERSION);
        manifest.extend_from_slice(&0u32.to_le_bytes());
        let slab_index_end = manifest.len();

        // §5.9 History (single build entry, timestamp 0 for determinism)
        let history_start = manifest.len();
        manifest.push(HISTORY_SECTION_VERSION);
        manifest.extend_from_slice(&1u32.to_le_bytes()); // 1 entry
        manifest.push(0x01); // op = build
        manifest.extend_from_slice(&0u64.to_le_bytes()); // timestamp = 0
        manifest.extend_from_slice(&0u32.to_le_bytes()); // input_count = 0
        manifest.extend_from_slice(&0u32.to_le_bytes()); // params_len = 0
        let history_end = manifest.len();

        // Compute Merkle root.
        let hashes = SectionHashes {
            metadata: metadata_hash,
            format_header: hash_section(&manifest[header_start..header_end]),
            feature_flags: hash_section(&manifest[flags_start..flags_end]),
            metadata_reference: hash_section(&manifest[meta_ref_start..meta_ref_end]),
            slab_index: hash_section(&manifest[slab_index_start..slab_index_end]),
            crypto_params: hash_empty_section(),
            ec_params: hash_empty_section(),
            dms_policy: hash_empty_section(),
            delta_linkage: hash_empty_section(),
            history: hash_section(&manifest[history_start..history_end]),
        };
        let merkle_root = compute_merkle_root(&hashes);

        WriteArtifact {
            bytes: manifest,
            merkle_root,
            inode_count,
            file_count: self.file_count,
            dir_count,
        }
    }

    fn encode_inode(&self, out: &mut Vec<u8>, inode: &PendingInode) {
        out.extend_from_slice(&inode.number.to_le_bytes());
        out.extend_from_slice(&inode.mode.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // uid
        out.extend_from_slice(&0u32.to_le_bytes()); // gid
        out.extend_from_slice(&inode.mtime_ns.to_le_bytes()); // mtime
        out.extend_from_slice(&inode.mtime_ns.to_le_bytes()); // ctime
        out.extend_from_slice(&1u32.to_le_bytes()); // nlink
        match &inode.content {
            PendingContent::Inline(data) => {
                out.push(0x04); // flags: INLINE_DATA
                let len = u32::try_from(data.len()).expect("data fits u32");
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(data);
            }
            PendingContent::Directory(entries) => {
                out.push(0x00); // flags: none
                let node = self
                    .dir_nodes
                    .iter()
                    .find(|n| n.entries == *entries)
                    .expect("directory node must exist");
                out.extend_from_slice(&node.hash);
            }
        }
    }
}

fn encode_dir_node(entries: &[(String, u64, u8)]) -> DirNode {
    let mut bytes = Vec::new();
    bytes.push(1u8); // node_version
    let count = u32::try_from(entries.len()).expect("entry count fits u32");
    bytes.extend_from_slice(&count.to_le_bytes());
    for (name, inode_number, entry_type) in entries {
        let name_bytes = name.as_bytes();
        let name_len = u32::try_from(name_bytes.len()).expect("name fits u32");
        bytes.extend_from_slice(&name_len.to_le_bytes());
        bytes.extend_from_slice(name_bytes);
        bytes.extend_from_slice(&inode_number.to_le_bytes());
        bytes.push(*entry_type);
    }
    let hash = hash_section(&bytes);
    DirNode {
        entries: entries.to_vec(),
        bytes,
        hash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use limnifs_core::ManifestCursor;

    #[test]
    fn write_empty_directory() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-empty", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert!(artifact.inode_count >= 1);
        assert_eq!(artifact.file_count, 0);
        assert_eq!(artifact.dir_count, 1);
        assert_ne!(artifact.merkle_root.as_bytes(), &[0u8; 32]);
    }

    #[test]
    fn write_small_file() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-file", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        std::fs::write(temp.join("hello.txt"), b"hello world").expect("write file");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert_eq!(artifact.file_count, 1);
        assert_eq!(artifact.dir_count, 1);
        assert_eq!(artifact.inode_count, 2); // 1 dir + 1 file
        assert_ne!(artifact.merkle_root.as_bytes(), &[0u8; 32]);
    }

    #[test]
    fn write_and_verify_roundtrip() {
        let temp = std::env::temp_dir().join(format!(
            "limnifs-write-test-{}-roundtrip",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        std::fs::write(temp.join("a.txt"), b"aaa").expect("write a");
        std::fs::write(temp.join("b.txt"), b"bbb").expect("write b");
        std::fs::create_dir_all(temp.join("sub")).expect("create sub");
        std::fs::write(temp.join("sub").join("c.txt"), b"ccc").expect("write c");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert_eq!(artifact.file_count, 3);
        assert_eq!(artifact.dir_count, 2);

        // Verify the manifest bytes parse correctly.
        let mut cursor = ManifestCursor::new(&artifact.bytes);
        let header = limnifs_core::parse_manifest_header(&mut cursor).expect("header");
        assert_eq!(header, ManifestHeader::current());
        let flags = limnifs_core::parse_feature_flags_section(&mut cursor).expect("flags");
        assert!(flags.is_empty());
        let meta_ref = limnifs_core::parse_metadata_reference(&mut cursor).expect("meta ref");
        assert!(meta_ref.is_inlined());
        let slab_index = limnifs_core::parse_slab_index(&mut cursor).expect("slab index");
        assert_eq!(slab_index.len(), 0);
        let history = limnifs_core::parse_history(&mut cursor).expect("history");
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn write_deterministic() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-det", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        std::fs::write(temp.join("x.txt"), b"xxx").expect("write x");

        let a1 = write_directory(&temp).expect("first write");
        let a2 = write_directory(&temp).expect("second write");
        std::fs::remove_dir_all(&temp).ok();

        assert_eq!(
            a1.bytes, a2.bytes,
            "same directory must produce identical bytes"
        );
        assert_eq!(a1.merkle_root, a2.merkle_root);
    }

    #[test]
    fn rejects_large_file() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-large", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let large_data = vec![0u8; INLINE_THRESHOLD + 1];
        std::fs::write(temp.join("big.bin"), &large_data).expect("write big");
        let result = write_directory(&temp);
        std::fs::remove_dir_all(&temp).ok();
        match result {
            Err(WriteError::FileTooLarge { .. }) => {}
            other => panic!("expected FileTooLarge, got {other:?}"),
        }
    }
}
