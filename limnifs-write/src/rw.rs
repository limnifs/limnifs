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
//!   ├── manifest_path: where the .lim lives
//!   ├── config: WriteConfig with write_codec + turnover defaults
//!   ├── state: parsed metadata blob + slab store (populated on open)
//!   ├── inode_map: path → inode map
//!   ├── pending_files: path → plaintext for staged writes
//!   ├── pending_history: operation log (add/update/delete)
//!   └── next_inode: allocator
//! ```
//!
//! ## Lifecycle
//!
//! 1. **Open** (`RwImage::open`): parse manifest, mmap slabs, build
//!    path index. Pending changes start empty.
//! 2. **Mutate**: `add_file` / `update_file` / `delete_file` stage
//!    changes. Files are kept as plaintext in memory.
//! 3. **Commit**: materialize the live tree (if opened) into a
//!    workspace `.scratch/` directory, overlay pending changes, and
//!    rebuild the image with the configured codecs.
//! 4. **Turnover**: re-build the current live tree with the
//!    configured (turnover) codecs, discarding pending changes. This
//!    is the hygiene operation that reclaims unreferenced drops.

#![allow(warnings)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use limnifs_core::codec;
use limnifs_core::slab_store::SlabStore;
use limnifs_core::{ContentHandle, ManifestCursor, MetadataBlob};

use crate::config::{ImageMode, WriteConfig};
use crate::WriteError;

/// An open read-write LimniFS image.
pub struct RwImage {
    manifest_path: PathBuf,
    config: WriteConfig,
    state: Option<OpenState>,
    /// Path → inode number map for fast lookups. Keys are absolute
    /// POSIX-style paths (with leading `/`).
    inode_map: HashMap<String, u64>,
    /// Files staged for write/update. Keys mirror `inode_map`.
    pending_files: HashMap<String, Vec<u8>>,
    /// Pending operations since open.
    pending_history: Vec<HistoryEntry>,
    /// Next available inode number.
    next_inode: u64,
}

/// State populated by `RwImage::open` so subsequent `commit` /
/// `turnover` calls can read the live tree without re-parsing.
struct OpenState {
    blob: MetadataBlob,
    root_inode: u64,
    slab_store: Option<SlabStore>,
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
    /// Open an existing image for read-write access. Parses the
    /// manifest, mmaps the slabs, and builds the path index.
    ///
    /// # Errors
    /// Returns [`WriteError`] if the manifest cannot be parsed or
    /// the slab files cannot be opened.
    pub fn open(path: &Path, config: WriteConfig) -> Result<Self, WriteError> {
        // Crash recovery: if a previous commit was interrupted
        // mid-swap, `<path>.new/` may still exist. The previous
        // manifest at `path` is intact (atomic swap was incomplete);
        // the `.new/` directory is garbage. Clean it up before
        // proceeding so the next commit's `write_artifact` doesn't
        // trip over a stale directory.
        cleanup_stale_swap_dir(path);

        let manifest_bytes = std::fs::read(path).map_err(WriteError::Io)?;

        let mut cursor = ManifestCursor::new(&manifest_bytes);
        let _ = limnifs_core::parse_manifest_header(&mut cursor).map_err(core_to_io)?;
        let _ = limnifs_core::parse_feature_flags_section(&mut cursor).map_err(core_to_io)?;
        let meta_ref = limnifs_core::parse_metadata_reference(&mut cursor).map_err(core_to_io)?;

        let blob_bytes: Vec<u8> = if let Some(inline) = meta_ref.inline_metadata.as_ref() {
            inline.clone()
        } else {
            let entry = meta_ref.locators.first().ok_or_else(|| {
                WriteError::Io(std::io::Error::other(
                    "metadata_reference has neither inline data nor locators",
                ))
            })?;
            let name = entry.uri.strip_prefix("file:").unwrap_or(&entry.uri);
            let sidecar = path.parent().unwrap_or_else(|| Path::new(".")).join(name);
            let wire_bytes = std::fs::read(&sidecar).map_err(WriteError::Io)?;
            if meta_ref.codec == 0 {
                wire_bytes
            } else {
                codec::decompress(meta_ref.codec, &wire_bytes, meta_ref.uncompressed_len)
                    .map_err(core_to_io)?
            }
        };

        let mut blob_cursor = ManifestCursor::new(&blob_bytes);
        let blob = limnifs_core::parse_metadata_blob(&mut blob_cursor).map_err(core_to_io)?;

        let slab_index = limnifs_core::parse_slab_index(&mut cursor).map_err(core_to_io)?;
        let slab_store = if slab_index.is_empty() {
            None
        } else {
            Some(SlabStore::load_mmap(path, &slab_index).map_err(core_to_io)?)
        };

        let root_inode = blob.root_inode_number().ok_or_else(|| {
            WriteError::Io(std::io::Error::other(
                "metadata blob: could not identify a unique root directory inode",
            ))
        })?;

        let path_index = blob.build_path_index();
        let next_inode = blob.inodes.iter().map(|i| i.number).max().unwrap_or(0) + 1;

        Ok(Self {
            manifest_path: path.to_path_buf(),
            config,
            state: Some(OpenState {
                blob,
                root_inode,
                slab_store,
            }),
            inode_map: path_index,
            pending_files: HashMap::new(),
            pending_history: Vec::new(),
            next_inode,
        })
    }

    /// Create a new empty RW image. The first `commit` produces the
    /// on-disk manifest + slabs.
    #[must_use]
    pub fn create_new(path: &Path, config: WriteConfig) -> Self {
        Self {
            manifest_path: path.to_path_buf(),
            config,
            state: None,
            inode_map: HashMap::new(),
            pending_files: HashMap::new(),
            pending_history: Vec::new(),
            next_inode: 1,
        }
    }

    /// Add a new file to the image. The plaintext is staged for the
    /// next `commit`; no I/O happens until then.
    ///
    /// # Errors
    /// Returns [`WriteError`] only if internal allocation fails.
    pub fn add_file(&mut self, path: &str, data: &[u8]) -> Result<u64, WriteError> {
        let inode = self.next_inode;
        self.next_inode += 1;
        let key = normalize_path(path);
        self.pending_files.insert(key.clone(), data.to_vec());
        self.inode_map.insert(key.clone(), inode);
        self.pending_history.push(HistoryEntry::Add {
            path: key,
            inode,
            size: data.len() as u64,
        });
        Ok(inode)
    }

    /// Update an existing file. The old inode is marked superseded;
    /// old drops remain in slabs until the next turnover.
    ///
    /// # Errors
    /// Returns [`WriteError`] if the path doesn't exist.
    pub fn update_file(&mut self, path: &str, data: &[u8]) -> Result<(), WriteError> {
        let key = normalize_path(path);
        let old_inode = *self.inode_map.get(&key).ok_or_else(|| {
            WriteError::Io(std::io::Error::other(format!("path not found: {path}")))
        })?;
        let new_inode = self.next_inode;
        self.next_inode += 1;
        self.pending_files.insert(key.clone(), data.to_vec());
        self.inode_map.insert(key.clone(), new_inode);
        self.pending_history.push(HistoryEntry::Update {
            path: key,
            old_inode,
            new_inode,
            size: data.len() as u64,
        });
        Ok(())
    }

    /// Delete a file. The inode is removed from the path index; old
    /// drops remain until the next turnover.
    ///
    /// # Errors
    /// Returns [`WriteError`] if the path doesn't exist.
    pub fn delete_file(&mut self, path: &str) -> Result<(), WriteError> {
        let key = normalize_path(path);
        let inode = self.inode_map.remove(&key).ok_or_else(|| {
            WriteError::Io(std::io::Error::other(format!("path not found: {path}")))
        })?;
        self.pending_files.remove(&key);
        self.pending_history
            .push(HistoryEntry::Delete { path: key, inode });
        Ok(())
    }

    /// Read a file's plaintext from the in-memory state. Only
    /// available for images that have been `open`ed.
    ///
    /// # Errors
    /// Returns [`WriteError`] if the image was not opened, the path
    /// is unknown, or the slab is missing/corrupt.
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>, WriteError> {
        let state = self.state.as_ref().ok_or_else(|| {
            WriteError::Io(std::io::Error::other("read_file: image was not opened"))
        })?;
        let key = normalize_path(path);
        let inode_num = *self.inode_map.get(&key).ok_or_else(|| {
            WriteError::Io(std::io::Error::other(format!("path not found: {path}")))
        })?;
        let inode = state.blob.inode_by_number(inode_num).ok_or_else(|| {
            WriteError::Io(std::io::Error::other(format!("inode {inode_num} missing")))
        })?;
        match &inode.content_handle {
            ContentHandle::InlineData(data) => Ok(data.clone()),
            ContentHandle::SliceMap(slices) => {
                let store = state.slab_store.as_ref().ok_or_else(|| {
                    WriteError::Io(std::io::Error::other(
                        "read_file: slice-backed file but no slab store",
                    ))
                })?;
                let mut out = Vec::new();
                for slice in slices {
                    let plaintext = store
                        .plaintext_for(slice.drop_id.as_bytes())
                        .ok_or_else(|| {
                            WriteError::Io(std::io::Error::other("drop not in any slab"))
                        })?
                        .map_err(core_to_io)?;
                    out.extend_from_slice(&plaintext);
                }
                Ok(out)
            }
            _ => Err(WriteError::Io(std::io::Error::other(
                "read_file: unsupported content handle",
            ))),
        }
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

    /// Commit pending changes. Materializes the live tree (if any),
    /// overlays pending writes, rebuilds the image with the
    /// configured codecs, and writes the new manifest + slabs.
    ///
    /// # Errors
    /// Returns [`WriteError`] on I/O or serialization failure.
    pub fn commit(&self) -> Result<crate::WriteArtifact, WriteError> {
        let staging = self.staging_dir();
        self.write_staging_tree(&staging)?;
        let artifact = crate::write_directory_with_config(&staging, &self.config)?;
        let _ = std::fs::remove_dir_all(&staging);
        self.write_artifact(&artifact)?;
        Ok(artifact)
    }

    /// Turnover: rebuild the current live tree with the configured
    /// codecs. Pending changes are dropped — this is a hygiene
    /// operation, not a commit.
    ///
    /// # Errors
    /// Returns [`WriteError`] on I/O or serialization failure.
    pub fn turnover(&self) -> Result<crate::WriteArtifact, WriteError> {
        let staging = self.staging_dir();
        if let Some(state) = &self.state {
            self.write_live_tree_only(state, &staging)?;
        } else {
            let _ = std::fs::remove_dir_all(&staging);
            std::fs::create_dir_all(&staging).map_err(WriteError::Io)?;
            self.write_pending_only(&staging)?;
        }
        let artifact = crate::write_directory_with_config(&staging, &self.config)?;
        let _ = std::fs::remove_dir_all(&staging);
        self.write_artifact(&artifact)?;
        Ok(artifact)
    }

    /// Build the staging tree: live tree (if opened) + pending
    /// changes (adds/updates overwriting live entries, deletes
    /// removing them).
    fn write_staging_tree(&self, staging: &Path) -> Result<(), WriteError> {
        let _ = std::fs::remove_dir_all(staging);
        std::fs::create_dir_all(staging).map_err(WriteError::Io)?;

        if let Some(state) = &self.state {
            self.write_live_tree(state, staging)?;
        }

        // Apply deletes for paths not covered by a subsequent
        // pending write.
        for entry in &self.pending_history {
            if let HistoryEntry::Delete { path, .. } = entry {
                if !self.pending_files.contains_key(path) {
                    let _ = std::fs::remove_file(staging.join(staging_relative(path)));
                }
            }
        }

        // Overlay pending writes.
        for (path, data) in &self.pending_files {
            let file_path = staging.join(staging_relative(path));
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent).map_err(WriteError::Io)?;
            }
            std::fs::write(&file_path, data).map_err(WriteError::Io)?;
        }
        Ok(())
    }

    fn write_pending_only(&self, staging: &Path) -> Result<(), WriteError> {
        for (path, data) in &self.pending_files {
            let file_path = staging.join(staging_relative(path));
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent).map_err(WriteError::Io)?;
            }
            std::fs::write(&file_path, data).map_err(WriteError::Io)?;
        }
        Ok(())
    }

    /// Turnover helper: write the live tree verbatim (no pending
    /// overlays) to `staging`.
    fn write_live_tree_only(&self, state: &OpenState, staging: &Path) -> Result<(), WriteError> {
        let _ = std::fs::remove_dir_all(staging);
        std::fs::create_dir_all(staging).map_err(WriteError::Io)?;
        self.write_live_tree(state, staging)?;
        Ok(())
    }

    /// Recursively walk the live tree and write each entry under
    /// `staging`. Delegates to the shared
    /// [`limnifs_core::live_tree::walk_live_tree`] with a
    /// [`FilesystemSink`].
    fn write_live_tree(&self, state: &OpenState, staging: &Path) -> Result<(), WriteError> {
        let mut sink =
            limnifs_core::live_tree::FilesystemSink::new(staging, state.slab_store.as_ref());
        limnifs_core::live_tree::walk_live_tree(&state.blob, state.root_inode, &mut sink)
            .map_err(core_to_io)
    }

    /// Persist the produced manifest + slabs to disk, replacing the
    /// previous files at the same paths.
    /// Persist the produced manifest + slabs to disk atomically.
    ///
    /// Files are written to `<manifest_path>.new/` then renamed into
    /// place. `rename(2)` is atomic for a single file on POSIX
    /// filesystems (APFS, ext4, btrfs, xfs); ordering the renames
    /// sidecar → slabs → manifest means a reader opening the manifest
    /// always sees a consistent snapshot (referenced slabs already
    /// exist).
    ///
    /// A crash mid-sequence leaves `<manifest_path>.new/` on disk;
    /// the next `RwImage::open` could detect and clean it up (TODO:
    /// `06-rw-crash-safety.md`).
    fn write_artifact(&self, artifact: &crate::WriteArtifact) -> Result<(), WriteError> {
        let parent = self
            .manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let staging = parent.join(format!(
            "{}.new",
            self.manifest_path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("image.lim"),
        ));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(WriteError::Io)?;

        // Write all files into staging first.
        let manifest_name = self
            .manifest_path
            .file_name()
            .map(std::ffi::OsString::from)
            .unwrap_or_else(|| std::ffi::OsString::from("image.lim"));
        let manifest_staging = staging.join(&manifest_name);
        std::fs::write(&manifest_staging, &artifact.bytes).map_err(WriteError::Io)?;

        let mut slab_names: Vec<std::ffi::OsString> = Vec::new();
        for slab in &artifact.slabs {
            let name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
            let os_name = std::ffi::OsString::from(name);
            std::fs::write(staging.join(&os_name), &slab.bytes).map_err(WriteError::Io)?;
            slab_names.push(os_name);
        }
        let sidecar_name: Option<std::ffi::OsString> =
            if let Some(sidecar) = &artifact.metadata_sidecar {
                let name = sidecar
                    .locator
                    .strip_prefix("file:")
                    .unwrap_or(&sidecar.locator);
                let os_name = std::ffi::OsString::from(name);
                std::fs::write(staging.join(&os_name), &sidecar.bytes).map_err(WriteError::Io)?;
                Some(os_name)
            } else {
                None
            };

        // Rename into place: sidecar → slabs → manifest. The manifest
        // is last so a reader never sees a manifest that references
        // missing slabs.
        if let Some(name) = &sidecar_name {
            rename_or_fallback(staging.join(name), parent.join(name))?;
        }
        for name in &slab_names {
            rename_or_fallback(staging.join(name), parent.join(name))?;
        }
        rename_or_fallback(manifest_staging, self.manifest_path.clone())?;

        // Cleanup staging directory (now empty).
        let _ = std::fs::remove_dir_all(&staging);
        Ok(())
    }

    /// Pick a workspace-local scratch directory for staging. Walks up
    /// from the manifest path to find a `Cargo.toml` with
    /// `[workspace]`; falls back to the manifest's parent directory.
    fn staging_dir(&self) -> PathBuf {
        let nonce = format!("{}-{}", std::process::id(), self.next_inode);
        let mut cur = self
            .manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        loop {
            if cur.join("Cargo.toml").is_file() {
                if std::fs::read_to_string(cur.join("Cargo.toml"))
                    .map(|s| s.contains("[workspace]"))
                    .unwrap_or(false)
                {
                    return cur.join(".scratch").join(format!("limnifs-rw-{nonce}"));
                }
            }
            if !cur.pop() {
                break;
            }
        }
        self.manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".scratch")
            .join(format!("limnifs-rw-{nonce}"))
    }
}

/// Normalize a user-supplied path to a leading-`/` form so it
/// matches `MetadataBlob::build_path_index` keys.
fn normalize_path(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        format!("/{trimmed}")
    }
}

/// Strip the leading `/` so a key can be safely joined onto a
/// staging root.
fn staging_relative(path: &str) -> &str {
    path.trim_start_matches('/')
}

/// `rename(2)` is atomic on POSIX filesystems when source and
/// destination are on the same filesystem. If they're not (e.g.
/// `/tmp` → `/`), `rename` fails with `EXDEV` — fall back to
/// `write` + `remove` so we still get the final state, just without
/// the cross-reader atomicity guarantee.
fn rename_or_fallback(from: PathBuf, to: PathBuf) -> Result<(), WriteError> {
    match std::fs::rename(&from, &to) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(18) => {
            // EXDEV: cross-device rename. Fall back.
            let bytes = std::fs::read(&from).map_err(WriteError::Io)?;
            std::fs::write(&to, &bytes).map_err(WriteError::Io)?;
            let _ = std::fs::remove_file(&from);
            Ok(())
        }
        Err(e) => Err(WriteError::Io(e)),
    }
}

fn core_to_io(e: limnifs_core::CoreError) -> WriteError {
    WriteError::Io(std::io::Error::other(format!("{e}")))
}

/// Detect and remove a stale `<path>.new/` directory left behind by
/// an interrupted commit. The previous manifest at `path` is intact
/// (atomic swap is incomplete by construction — `write_artifact`
/// renames the manifest last); the `.new/` is garbage.
///
/// Logs nothing on success; silently ignores missing directory. If
/// the directory exists but cannot be removed (e.g. permissions),
/// the next commit's `write_artifact` will fail with a clearer
/// error when it tries to recreate the directory.
fn cleanup_stale_swap_dir(path: &Path) {
    let Some(name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
        return;
    };
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stale = parent.join(format!("{name}.new"));
    if stale.is_dir() {
        let _ = std::fs::remove_dir_all(&stale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile;

    #[test]
    fn open_cleans_up_stale_new_directory() {
        // Simulate a crashed previous commit: image exists, plus a
        // stale <image>.new/ directory. RwImage::open must remove
        // the stale directory so the next commit doesn't trip.
        let workdir = std::env::temp_dir().join(format!(
            "limnifs-crash-recovery-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
        ));
        let _ = std::fs::remove_dir_all(&workdir);
        std::fs::create_dir_all(&workdir).expect("mkdir");

        // Write a minimal valid image.
        std::fs::write(workdir.join("data.txt"), b"alpha").expect("src");
        let manifest = workdir.join("image.lim");
        let artifact =
            crate::write_directory_with_config(&workdir, &profile::balanced()).expect("write");
        std::fs::write(&manifest, &artifact.bytes).expect("manifest");
        for slab in &artifact.slabs {
            let name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
            std::fs::write(workdir.join(name), &slab.bytes).expect("slab");
        }
        if let Some(sidecar) = &artifact.metadata_sidecar {
            let name = sidecar
                .locator
                .strip_prefix("file:")
                .unwrap_or(&sidecar.locator);
            std::fs::write(workdir.join(name), &sidecar.bytes).expect("sidecar");
        }

        // Simulate a crash: create <image>.new/ with garbage.
        let stale = workdir.join("image.lim.new");
        std::fs::create_dir_all(&stale).expect("mkdir stale");
        std::fs::write(stale.join("partial.lim"), b"garbage from crashed commit").expect("garbage");
        assert!(stale.is_dir(), "stale dir exists before open");

        // Open should clean it up.
        let _image = RwImage::open(&manifest, profile::balanced()).expect("open");
        assert!(!stale.exists(), "stale dir removed by open");

        let _ = std::fs::remove_dir_all(&workdir);
    }

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

    /// Helper: write `files` into a fresh staging dir, build an
    /// image with the given config, and return the manifest path.
    fn write_initial(files: &[(&str, &[u8])], config: &WriteConfig) -> PathBuf {
        let staging = std::env::temp_dir().join(format!(
            "limnifs-rw-test-init-{}-{}",
            std::process::id(),
            rand_u64()
        ));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).expect("mkdir staging");
        for (name, data) in files {
            let path = staging.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("mkdir parent");
            }
            std::fs::write(&path, data).expect("write file");
        }
        let manifest = staging.join("image.lim");
        let artifact = crate::write_directory_with_config(&staging, config).expect("write");
        std::fs::write(&manifest, &artifact.bytes).expect("write manifest");
        for slab in &artifact.slabs {
            let name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
            std::fs::write(staging.join(name), &slab.bytes).expect("write slab");
        }
        if let Some(sidecar) = &artifact.metadata_sidecar {
            let name = sidecar
                .locator
                .strip_prefix("file:")
                .unwrap_or(&sidecar.locator);
            std::fs::write(staging.join(name), &sidecar.bytes).expect("write sidecar");
        }
        manifest
    }

    /// Tiny PRNG to avoid pulling the `rand` crate just for a
    /// non-colliding nonce in tests.
    fn rand_u64() -> u64 {
        use std::cell::Cell;
        use std::time::{SystemTime, UNIX_EPOCH};
        thread_local!(static SEED: Cell<u64> = {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            Cell::new(nanos ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        });
        SEED.with(|s| {
            let mut x = s.get();
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            s.set(x);
            x
        })
    }

    #[test]
    fn rw_image_open_round_trip() {
        let config = profile::balanced();
        let manifest = write_initial(
            &[("hello.txt", b"hello world"), ("dir/note.txt", b"nested")],
            &config,
        );
        let image = RwImage::open(&manifest, profile::balanced()).expect("open");
        assert_eq!(
            image.read_file("hello.txt").expect("read hello"),
            b"hello world"
        );
        assert_eq!(
            image.read_file("dir/note.txt").expect("read note"),
            b"nested"
        );
    }

    #[test]
    fn rw_image_commit_adds_file() {
        let config = profile::balanced();
        let manifest = write_initial(&[("a.txt", b"alpha")], &config);

        let mut image = RwImage::open(&manifest, profile::balanced()).expect("open");
        image.add_file("b.txt", b"beta").expect("add");
        let _ = image.commit().expect("commit");

        let reread = RwImage::open(&manifest, profile::balanced()).expect("reopen");
        assert_eq!(reread.read_file("a.txt").expect("read a"), b"alpha");
        assert_eq!(reread.read_file("b.txt").expect("read b"), b"beta");
    }

    #[test]
    fn rw_image_commit_updates_and_deletes() {
        let config = profile::balanced();
        let manifest = write_initial(&[("a.txt", b"alpha"), ("b.txt", b"beta")], &config);

        let mut image = RwImage::open(&manifest, profile::balanced()).expect("open");
        image.update_file("a.txt", b"alpha2").expect("update");
        image.delete_file("b.txt").expect("delete");
        let _ = image.commit().expect("commit");

        let reread = RwImage::open(&manifest, profile::balanced()).expect("reopen");
        assert_eq!(reread.read_file("a.txt").expect("read a"), b"alpha2");
        assert!(reread.read_file("b.txt").is_err(), "b.txt must be gone");
    }

    #[test]
    fn rw_image_turnover_preserves_tree() {
        let config = profile::max_write();
        let manifest = write_initial(
            &[("hello.txt", b"hello world"), ("dir/note.txt", b"nested")],
            &config,
        );

        let image = RwImage::open(&manifest, profile::max_write()).expect("open");
        let _ = image.turnover().expect("turnover");

        let reread = RwImage::open(&manifest, profile::max_write()).expect("reopen");
        assert_eq!(
            reread.read_file("hello.txt").expect("read hello"),
            b"hello world"
        );
        assert_eq!(
            reread.read_file("dir/note.txt").expect("read note"),
            b"nested"
        );
    }
}
