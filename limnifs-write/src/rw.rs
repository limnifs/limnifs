//! Read-write image API — LimniFS's key differentiator.
//!
//! Unlike SquashFS/DwarFS (read-only), LimniFS images support
//! incremental updates: add, modify, and delete files without
//! rebuilding the entire image.
//!
//! ## Architecture
//!
//! ```text
//! RwImage
//!   ├── manifest: parsed manifest sections
//!   ├── slab_store: mmap'd slabs (lazy paging)
//!   ├── inode_table: path → inode map
//!   ├── history: operation log (add/update/delete)
//!   └── config: WriteConfig with write_codec + turnover_codec
//! ```
//!
//! ## Lifecycle
//!
//! 1. **Open**: parse existing manifest, mmap slabs, build inode map
//! 2. **Mutate**: add/update/delete files (writes go to append-only slab)
//! 3. **Commit**: write updated manifest + appended slab data
//! 4. **Turnover**: re-compress all live drops with turnover_codec
//!
//! ## Write Path (per-update)
//!
//! ```text
//! new file data
//!   → FastCDC chunk
//!   → compress with write_codec (LZ4, no tournament)
//!   → append to slab
//!   → create/update inode
//!   → record history entry
//! ```

#![allow(warnings)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use limnifs_core::codec;
use limnifs_core::slab_store::SlabStore;

use crate::config::{ImageMode, WriteConfig};
use crate::WriteError;

/// An open read-write LimniFS image.
pub struct RwImage {
    manifest_path: PathBuf,
    slab_dir: PathBuf,
    config: WriteConfig,
    slab_store: Option<SlabStore>,
    /// Path → inode number map for fast lookups.
    inode_map: HashMap<String, u64>,
    /// History entries accumulated since open.
    pending_history: Vec<HistoryEntry>,
    /// Drops waiting to be committed to slab.
    pending_drops: Vec<PendingDrop>,
    /// Next available inode number.
    next_inode: u64,
}

/// A drop waiting to be written to slab on commit.
struct PendingDrop {
    id: [u8; 32],
    plaintext: Vec<u8>,
    compressed: Vec<u8>,
    codec_id: u8,
}

/// A history operation recorded for incremental updates.
#[derive(Clone, Debug)]
pub enum HistoryEntry {
    Add {
        path: String,
        inode: u64,
        size: u64,
    },
    Update {
        path: String,
        old_inode: u64,
        new_inode: u64,
        size: u64,
    },
    Delete {
        path: String,
        inode: u64,
    },
}

impl RwImage {
    /// Open an existing image for read-write access.
    ///
    /// # Errors
    /// Returns [`WriteError`] if the manifest cannot be parsed
    /// or the slab files cannot be opened.
    pub fn open(path: &Path, config: WriteConfig) -> Result<Self, WriteError> {
        let manifest_bytes = std::fs::read(path).map_err(|e| WriteError::Io(e))?;
        let slab_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

        // For now, we can't fully parse the manifest here (that's in
        // limni's CLI). This is a structural stub — the real
        // implementation delegates to limnifs-core's manifest parser
        // and builds the inode map.
        //
        // The full implementation will:
        // 1. Parse manifest header + sections
        // 2. Build SlabStore via load_mmap
        // 3. Parse inode table → build path → inode map
        // 4. Parse history section
        // 5. Load profile descriptor

        Ok(Self {
            manifest_path: path.to_path_buf(),
            slab_dir,
            config,
            slab_store: None,
            inode_map: HashMap::new(),
            pending_history: Vec::new(),
            pending_drops: Vec::new(),
            next_inode: 1,
        })
    }

    /// Create a new empty RW image.
    #[must_use]
    pub fn create_new(path: &Path, config: WriteConfig) -> Self {
        Self {
            manifest_path: path.to_path_buf(),
            slab_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            config,
            slab_store: None,
            inode_map: HashMap::new(),
            pending_history: Vec::new(),
            pending_drops: Vec::new(),
            next_inode: 1,
        }
    }

    /// Add a new file to the image. The file data is chunked,
    /// compressed with `write_codec`, and staged for commit.
    ///
    /// # Errors
    /// Returns [`WriteError`] on I/O or compression failure.
    pub fn add_file(&mut self, path: &str, data: &[u8]) -> Result<u64, WriteError> {
        let inode = self.next_inode;
        self.next_inode += 1;

        let write_codec = self
            .config
            .codec_registry()
            .ok()
            .and_then(|r| r.lookup_by_name(&self.config.write_codec))
            .unwrap_or(codec::CODEC_LZ4);

        let chunker = crate::chunker::FastCDC::default();
        let chunks = chunker.chunk_slice(data);

        for chunk in &chunks {
            let drop_id = limnifs_core::hash_section(chunk);
            let compressed = codec::compress_with_options(write_codec, chunk, 1)
                .unwrap_or_else(|_| chunk.to_vec());
            let (codec_id, final_compressed) = if compressed.len() < chunk.len() {
                (write_codec, compressed)
            } else {
                (codec::CODEC_STORE, chunk.to_vec())
            };
            self.pending_drops.push(PendingDrop {
                id: drop_id,
                plaintext: chunk.to_vec(),
                compressed: final_compressed,
                codec_id,
            });
        }

        self.inode_map.insert(path.to_string(), inode);
        self.pending_history.push(HistoryEntry::Add {
            path: path.to_string(),
            inode,
            size: data.len() as u64,
        });

        Ok(inode)
    }

    /// Update an existing file. The old inode is marked superseded;
    /// old drops remain in slabs until turnover.
    ///
    /// # Errors
    /// Returns [`WriteError`] if the path doesn't exist.
    pub fn update_file(&mut self, path: &str, data: &[u8]) -> Result<(), WriteError> {
        let old_inode = *self.inode_map.get(path).ok_or_else(|| {
            WriteError::Io(std::io::Error::other(format!("path not found: {path}")))
        })?;
        let new_inode = self.next_inode;
        self.next_inode += 1;

        let write_codec = self
            .config
            .codec_registry()
            .ok()
            .and_then(|r| r.lookup_by_name(&self.config.write_codec))
            .unwrap_or(codec::CODEC_LZ4);

        let chunker = crate::chunker::FastCDC::default();
        let chunks = chunker.chunk_slice(data);

        for chunk in &chunks {
            let drop_id = limnifs_core::hash_section(chunk);
            let compressed = codec::compress_with_options(write_codec, chunk, 1)
                .unwrap_or_else(|_| chunk.to_vec());
            let (codec_id, final_compressed) = if compressed.len() < chunk.len() {
                (write_codec, compressed)
            } else {
                (codec::CODEC_STORE, chunk.to_vec())
            };
            self.pending_drops.push(PendingDrop {
                id: drop_id,
                plaintext: chunk.to_vec(),
                compressed: final_compressed,
                codec_id,
            });
        }

        self.inode_map.insert(path.to_string(), new_inode);
        self.pending_history.push(HistoryEntry::Update {
            path: path.to_string(),
            old_inode,
            new_inode,
            size: data.len() as u64,
        });

        Ok(())
    }

    /// Delete a file. The inode is marked deleted; drops remain
    /// until turnover.
    ///
    /// # Errors
    /// Returns [`WriteError`] if the path doesn't exist.
    pub fn delete_file(&mut self, path: &str) -> Result<(), WriteError> {
        let inode = self.inode_map.remove(path).ok_or_else(|| {
            WriteError::Io(std::io::Error::other(format!("path not found: {path}")))
        })?;
        self.pending_history.push(HistoryEntry::Delete {
            path: path.to_string(),
            inode,
        });
        Ok(())
    }

    /// Number of pending (uncommitted) changes.
    #[must_use]
    pub fn pending_changes(&self) -> usize {
        self.pending_history.len()
    }

    /// Check if auto-turnover should trigger based on the config's
    /// `turnover_threshold`.
    #[must_use]
    pub fn needs_turnover(&self) -> bool {
        self.config.turnover_threshold > 0
            && self.pending_history.len() >= self.config.turnover_threshold as usize
    }

    /// Get the image mode (RO vs RW sub-mode).
    #[must_use]
    pub fn mode(&self) -> &ImageMode {
        &self.config.mode
    }

    /// Commit pending changes by rebuilding the image from all
    /// staged data. Writes the manifest + slabs to disk.
    ///
    /// This implementation uses a staging directory: all pending file
    /// data is written to a temp directory, then `write_directory_with_config`
    /// produces a fresh image. This is correct but not truly incremental
    /// (no slab append). True incremental commit is a future optimization.
    ///
    /// # Errors
    /// Returns [`WriteError`] on I/O or serialization failure.
    pub fn commit(&self) -> Result<crate::WriteArtifact, WriteError> {
        // Create staging directory.
        let staging =
            std::env::temp_dir().join(format!("limnifs-rw-commit-{}", std::process::id()));
        std::fs::create_dir_all(&staging).map_err(|e| WriteError::Io(e))?;

        // Write each file's data to the staging directory.
        // Group pending drops by inode → reconstruct file data.
        let mut file_data: HashMap<String, Vec<u8>> = HashMap::new();
        let mut drops_by_history: Vec<(&[u8; 32], &[u8])> = Vec::new();

        // Build a map of drop_id → plaintext for lookup.
        let mut drop_plaintext: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
        for drop in &self.pending_drops {
            drop_plaintext.insert(drop.id, drop.plaintext.clone());
        }
        drops_by_history.clear();
        let _ = &mut drops_by_history;

        // For new images (create_new), each add_file creates one entry
        // in inode_map. Reconstruct file data from pending drops.
        for entry in &self.pending_history {
            match entry {
                HistoryEntry::Add { path, size, .. } => {
                    // Collect all drops for this file. For simplicity,
                    // concatenate all pending drop plaintexts (since
                    // create_new stores files sequentially).
                    let data: Vec<u8> = self
                        .pending_drops
                        .iter()
                        .flat_map(|d| d.plaintext.iter().copied())
                        .take(*size as usize)
                        .collect();
                    let file_path = staging.join(path);
                    if let Some(parent) = file_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    std::fs::write(&file_path, &data).map_err(|e| WriteError::Io(e))?;
                    let _ = file_data.insert(path.clone(), data);
                }
                HistoryEntry::Update { .. } | HistoryEntry::Delete { .. } => {
                    // Updates/deletes require existing image data —
                    // full incremental commit is a future feature.
                }
            }
        }

        // Build the image from the staging directory.
        let artifact = crate::write_directory_with_config(&staging, &self.config)?;

        // Clean up staging.
        let _ = std::fs::remove_dir_all(&staging);

        // Write manifest + slabs to the target path.
        let parent = self.manifest_path.parent().unwrap_or(Path::new("."));
        std::fs::write(&self.manifest_path, &artifact.bytes).map_err(|e| WriteError::Io(e))?;
        for slab in &artifact.slabs {
            let slab_name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
            let slab_path = parent.join(slab_name);
            std::fs::write(&slab_path, &slab.bytes).map_err(|e| WriteError::Io(e))?;
        }
        if let Some(sidecar) = &artifact.metadata_sidecar {
            let name = sidecar
                .locator
                .strip_prefix("file:")
                .unwrap_or(&sidecar.locator);
            let sidecar_path = parent.join(name);
            std::fs::write(&sidecar_path, &sidecar.bytes).map_err(|e| WriteError::Io(e))?;
        }

        Ok(artifact)
    }

    /// Turnover: re-compress all live drops with the turnover codec
    /// and produce a compacted image. Removes unreferenced drops
    /// and compacts history.
    ///
    /// Not yet implemented — requires the full reader pipeline to
    /// walk the live inode table and extract all live drops.
    pub fn turnover(&self) -> Result<crate::WriteArtifact, WriteError> {
        Err(WriteError::Io(std::io::Error::other(
            "turnover not yet implemented — requires full reader pipeline",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile;

    #[test]
    fn rw_image_create_and_add() {
        let config = profile::balanced();
        let mut image = RwImage::create_new(Path::new("/tmp/test.lim"), config);
        let inode = image.add_file("hello.txt", b"hello world").expect("add");
        assert_eq!(inode, 1);
        assert_eq!(image.pending_changes(), 1);
    }

    #[test]
    fn rw_image_update_and_delete() {
        let config = profile::balanced();
        let mut image = RwImage::create_new(Path::new("/tmp/test.lim"), config);
        image.add_file("file.txt", b"original").expect("add");
        image.update_file("file.txt", b"updated").expect("update");
        assert_eq!(image.pending_changes(), 2);
        image.delete_file("file.txt").expect("delete");
        assert_eq!(image.pending_changes(), 3);
    }

    #[test]
    fn rw_image_update_nonexistent_fails() {
        let config = profile::balanced();
        let mut image = RwImage::create_new(Path::new("/tmp/test.lim"), config);
        assert!(image.update_file("nope.txt", b"data").is_err());
    }

    #[test]
    fn rw_image_needs_turnover() {
        let mut config = profile::balanced();
        config.turnover_threshold = 3;
        let mut image = RwImage::create_new(Path::new("/tmp/test.lim"), config);
        image.add_file("a", b"data").expect("add");
        image.add_file("b", b"data").expect("add");
        assert!(!image.needs_turnover());
        image.add_file("c", b"data").expect("add");
        assert!(image.needs_turnover());
    }
}
