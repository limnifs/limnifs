//! `LimniFS` writer pipeline — directory tree to `.lim` image.
//!
//! The writer takes a real directory tree and produces a valid `.lim`
//! manifest artifact with inlined metadata. Files at or below the
//! inline threshold (4 KiB) are stored as inline data in their inodes;
//! larger files are stored as drops packed into a single slab.
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

use std::collections::HashMap;
use std::path::Path;

use limnifs_core::{
    compute_merkle_root, hash_empty_section, hash_section, ManifestHeader, SectionHashes,
    FEATURE_FLAGS_SECTION_VERSION, HISTORY_SECTION_VERSION, METADATA_REFERENCE_SECTION_VERSION,
    SLAB_INDEX_SECTION_VERSION,
};
use limnifs_format::{ManifestRoot, SlabId};

/// Inline-data threshold: files at or below this size get inline data
/// in their inode. Larger files are stored as drops in a slab.
pub const INLINE_THRESHOLD: usize = 4096;

/// Result of writing a directory tree.
#[derive(Clone, Debug)]
pub struct WriteArtifact {
    pub bytes: Vec<u8>,
    pub merkle_root: ManifestRoot,
    pub slab_bytes: Option<Vec<u8>>,
    pub slab_locator: Option<String>,
    pub inode_count: usize,
    pub file_count: usize,
    pub dir_count: usize,
    pub drop_count: usize,
}

/// Error during writing.
#[derive(Debug)]
pub enum WriteError {
    Io(std::io::Error),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
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
/// with inlined metadata. Files at or below [`INLINE_THRESHOLD`] bytes
/// are stored inline; larger files are packed into a single slab as
/// content-addressed drops.
///
/// # Errors
///
/// Returns [`WriteError::Io`] for filesystem errors.
pub fn write_directory(root: &Path) -> Result<WriteArtifact, WriteError> {
    let mut ctx = WriteContext::new();
    let root_inode_number = ctx.walk(root)?;
    ctx.root_inode_number = root_inode_number;
    let artifact = ctx.assemble();
    Ok(artifact)
}

struct PendingDrop {
    id: [u8; 32],
    plaintext: Vec<u8>,
    offset_in_window: u32,
}

struct PendingInode {
    number: u64,
    mode: u32,
    mtime_ns: u64,
    content: PendingContent,
}

enum PendingContent {
    Inline(Vec<u8>),
    DropBacked {
        drop_id: [u8; 32],
        file_len: u64,
        offset_in_window: u32,
        len_in_window: u32,
    },
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
    drops: Vec<PendingDrop>,
    drop_index: HashMap<[u8; 32], (u32, u32)>,
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
            drops: Vec::new(),
            drop_index: HashMap::new(),
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
            let data = std::fs::read(path)?;
            let file_len = data.len();

            if file_len <= INLINE_THRESHOLD {
                self.inodes.push(PendingInode {
                    number: inode_number,
                    mode: 0o100_644,
                    mtime_ns,
                    content: PendingContent::Inline(data),
                });
            } else {
                let drop_id = hash_section(&data);
                let (offset, len) = if let Some(&existing) = self.drop_index.get(&drop_id) {
                    existing
                } else {
                    let offset = self.drops.iter().map(PendingDrop::len_in_window).sum::<u32>();
                    let len = u32::try_from(file_len).expect("file fits u32");
                    self.drops.push(PendingDrop {
                        id: drop_id,
                        plaintext: data,
                        offset_in_window: offset,
                    });
                    self.drop_index.insert(drop_id, (offset, len));
                    (offset, len)
                };
                self.inodes.push(PendingInode {
                    number: inode_number,
                    mode: 0o100_644,
                    mtime_ns,
                    content: PendingContent::DropBacked {
                        drop_id,
                        file_len: file_len as u64,
                        offset_in_window: offset,
                        len_in_window: len,
                    },
                });
            }
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
        let drop_count = self.drops.len();

        let (slab_bytes, slab_id, slab_locator) = if self.drops.is_empty() {
            (None, None, None)
        } else {
            let (bytes, id) = encode_slab(&self.drops);
            let locator = "file:slab-0.bin".to_owned();
            (Some(bytes), Some(id), Some(locator))
        };

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

        let mut manifest = Vec::new();

        let header_start = manifest.len();
        manifest.extend_from_slice(&ManifestHeader::current().to_bytes());
        let header_end = manifest.len();

        let flags_start = manifest.len();
        manifest.push(FEATURE_FLAGS_SECTION_VERSION);
        manifest.extend_from_slice(&0u32.to_le_bytes());
        let flags_end = manifest.len();

        let meta_ref_start = manifest.len();
        manifest.push(METADATA_REFERENCE_SECTION_VERSION);
        let metadata_hash = hash_section(&metadata_blob);
        manifest.extend_from_slice(&metadata_hash);
        manifest.extend_from_slice(&0u32.to_le_bytes());
        let inline_len = u32::try_from(metadata_blob.len()).expect("metadata fits u32");
        manifest.extend_from_slice(&inline_len.to_le_bytes());
        manifest.extend_from_slice(&metadata_blob);
        let meta_ref_end = manifest.len();

        let slab_index_start = manifest.len();
        manifest.push(SLAB_INDEX_SECTION_VERSION);
        if let (Some(id), Some(loc)) = (&slab_id, &slab_locator) {
            manifest.extend_from_slice(&1u32.to_le_bytes());
            manifest.extend_from_slice(&id.to_bytes());
            manifest.extend_from_slice(&1u32.to_le_bytes());
            let loc_bytes = loc.as_bytes();
            let loc_len = u32::try_from(loc_bytes.len()).expect("locator fits u32");
            manifest.extend_from_slice(&loc_len.to_le_bytes());
            manifest.extend_from_slice(loc_bytes);
        } else {
            manifest.extend_from_slice(&0u32.to_le_bytes());
        }
        let slab_index_end = manifest.len();

        let history_start = manifest.len();
        manifest.push(HISTORY_SECTION_VERSION);
        manifest.extend_from_slice(&1u32.to_le_bytes());
        manifest.push(0x01);
        manifest.extend_from_slice(&0u64.to_le_bytes());
        manifest.extend_from_slice(&0u32.to_le_bytes());
        manifest.extend_from_slice(&0u32.to_le_bytes());
        let history_end = manifest.len();

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
            slab_bytes,
            slab_locator,
            inode_count,
            file_count: self.file_count,
            dir_count,
            drop_count,
        }
    }

    fn encode_inode(&self, out: &mut Vec<u8>, inode: &PendingInode) {
        out.extend_from_slice(&inode.number.to_le_bytes());
        out.extend_from_slice(&inode.mode.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&inode.mtime_ns.to_le_bytes());
        out.extend_from_slice(&inode.mtime_ns.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        match &inode.content {
            PendingContent::Inline(data) => {
                out.push(0x04);
                let len = u32::try_from(data.len()).expect("data fits u32");
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(data);
            }
            PendingContent::DropBacked {
                drop_id,
                file_len,
                offset_in_window,
                len_in_window,
            } => {
                out.push(0x00);
                out.extend_from_slice(&1u32.to_le_bytes());
                out.extend_from_slice(&0u64.to_le_bytes());
                out.extend_from_slice(&file_len.to_le_bytes());
                out.extend_from_slice(drop_id);
                out.extend_from_slice(&offset_in_window.to_le_bytes());
                out.extend_from_slice(&len_in_window.to_le_bytes());
            }
            PendingContent::Directory(entries) => {
                out.push(0x00);
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

impl PendingDrop {
    fn len_in_window(&self) -> u32 {
        u32::try_from(self.plaintext.len()).expect("drop plaintext fits u32")
    }
}

fn encode_dir_node(entries: &[(String, u64, u8)]) -> DirNode {
    let mut bytes = Vec::new();
    bytes.push(1u8);
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

fn encode_slab(drops: &[PendingDrop]) -> (Vec<u8>, SlabId) {
    let mut drop_records = Vec::new();
    let mut solid_window = Vec::new();

    for drop in drops {
        let plaintext_len = drop.len_in_window();
        drop_records.extend_from_slice(&drop.id);
        drop_records.extend_from_slice(&plaintext_len.to_le_bytes());
        drop_records.extend_from_slice(&[0x00, 0x00, 0x00]);
        drop_records.push(0x00);
        drop_records.extend_from_slice(&drop.offset_in_window.to_le_bytes());
        drop_records.extend_from_slice(&plaintext_len.to_le_bytes());
        solid_window.extend_from_slice(&drop.plaintext);
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
    slab_bytes.push(0x00);
    slab_bytes.push(0x00);
    slab_bytes.extend_from_slice(&slab_content);

    (slab_bytes, slab_id)
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
        assert!(artifact.slab_bytes.is_none());
    }

    #[test]
    fn write_small_file_inline() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-small", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        std::fs::write(temp.join("hello.txt"), b"hello world").expect("write file");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert_eq!(artifact.file_count, 1);
        assert!(artifact.slab_bytes.is_none());
        assert_eq!(artifact.drop_count, 0);
    }

    #[test]
    fn write_large_file_uses_slab() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-large", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let large_data = vec![0xABu8; INLINE_THRESHOLD + 100];
        std::fs::write(temp.join("big.bin"), &large_data).expect("write big");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert_eq!(artifact.drop_count, 1);
        assert!(artifact.slab_bytes.is_some());
        assert!(artifact.slab_locator.is_some());
    }

    #[test]
    fn write_mixed_inline_and_large() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-mix", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        std::fs::write(temp.join("small.txt"), b"tiny").expect("write small");
        std::fs::write(temp.join("large.bin"), vec![0xCDu8; INLINE_THRESHOLD * 2])
            .expect("write large");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert_eq!(artifact.file_count, 2);
        assert_eq!(artifact.drop_count, 1);
        assert!(artifact.slab_bytes.is_some());
    }

    #[test]
    fn deduplicates_identical_large_files() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-dedup", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let data = vec![0x77u8; INLINE_THRESHOLD + 10];
        std::fs::write(temp.join("a.bin"), &data).expect("write a");
        std::fs::write(temp.join("b.bin"), &data).expect("write b");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();
        assert_eq!(artifact.drop_count, 1);
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

        let mut cursor = ManifestCursor::new(&artifact.bytes);
        limnifs_core::parse_manifest_header(&mut cursor).expect("header");
        limnifs_core::parse_feature_flags_section(&mut cursor).expect("flags");
        let meta_ref = limnifs_core::parse_metadata_reference(&mut cursor).expect("meta ref");
        assert!(meta_ref.is_inlined());
        let slab_index = limnifs_core::parse_slab_index(&mut cursor).expect("slab index");
        assert_eq!(slab_index.len(), 0);
        limnifs_core::parse_history(&mut cursor).expect("history");
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

        assert_eq!(a1.bytes, a2.bytes);
        assert_eq!(a1.merkle_root, a2.merkle_root);
    }

    #[test]
    fn slab_parses_correctly() {
        let temp =
            std::env::temp_dir().join(format!("limnifs-write-test-{}-slab", std::process::id()));
        std::fs::create_dir_all(&temp).expect("create temp dir");
        std::fs::write(temp.join("big.bin"), vec![0x11u8; INLINE_THRESHOLD + 1])
            .expect("write big");
        let artifact = write_directory(&temp).expect("write succeeds");
        std::fs::remove_dir_all(&temp).ok();

        let slab_bytes = artifact.slab_bytes.as_ref().expect("slab exists");
        let mut cursor = ManifestCursor::new(slab_bytes);
        let slab_header = limnifs_core::parse_slab_header(&mut cursor).expect("slab header parses");
        assert_eq!(slab_header.format_version, 1);
        assert!(!slab_header.is_sealed());
        assert!(!slab_header.has_erasure_coding());

        let drop_record =
            limnifs_core::parse_drop_record(&mut cursor, &slab_header).expect("drop record parses");
        assert_eq!(drop_record.plaintext_len as usize, INLINE_THRESHOLD + 1);
    }
}
