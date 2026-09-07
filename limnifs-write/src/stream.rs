//! Streaming multi-file writer — the no-materialisation seam.
//!
//! `write_stream` packs ONE named stream; [`StreamWriter`]
//! generalises it to a whole tree of streams (tar archives, pipe
//! bundles, network feeds) without ever touching the filesystem:
//! entries are chunked straight off their readers via
//! [`Chunker::chunk_reader`], whose internal buffering is bounded by
//! the chunker's max chunk size plus one read buffer.
//!
//! ## Tree construction
//!
//! Entries arrive in arbitrary order; the tree is a nested
//! `BTreeMap` so directory entries materialise name-sorted at
//! `finish`. File and symlink inodes are allocated (and pushed) in
//! arrival order; directory inodes are allocated parent-first
//! during `finish`. The numbering therefore differs from a
//! directory pack of the same tree (where the DFS orders
//! allocation) — the format only requires unique numbers — but the
//! same entry sequence always produces byte-identical images.
//!
//! [`Chunker::chunk_reader`]: crate::chunker::Chunker::chunk_reader

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;

use crate::chunker::Chunker;
use crate::classifier;
use crate::config::WriteConfig;
use crate::{
    encode_dir_node, hash_section, PendingContent, PendingFile, PendingInode, TournamentSpec,
    WriteArtifact, WriteContext, WriteError,
};

/// Codec setup shared by every entry of one stream write. Built
/// once at construction so per-entry cost is pure chunk + compress.
struct StreamCodecs {
    chunker: crate::chunker::ParallelFastCDC,
    classifier: classifier::Classifier,
    text_codec: u8,
    binary_codec: u8,
    tunables: limnifs_core::codec::CodecTunables,
    tournament: TournamentSpec,
}

impl StreamCodecs {
    fn from_config(
        chunker: crate::chunker::ParallelFastCDC,
        classifier: classifier::Classifier,
        config: &WriteConfig,
    ) -> Result<Self, WriteError> {
        let registry = config
            .codec_registry()
            .map_err(|e| WriteError::Io(std::io::Error::other(format!("codec registry: {e}"))))?;
        let tournament_codec_ids: Vec<u8> = config
            .tournament
            .codecs
            .iter()
            .filter_map(|n| registry.lookup_by_name(n))
            .collect();
        Ok(Self {
            chunker,
            classifier,
            text_codec: config.text_codec_id().unwrap_or(0x04),
            binary_codec: config.binary_codec_id().unwrap_or(0x01),
            tunables: config.to_core_tunables(),
            tournament: TournamentSpec {
                codec_ids: tournament_codec_ids,
                min_size: config.tournament.min_size_threshold as usize,
                skip_for_binary: config.tournament.skip_for_binary,
                short_circuit_permille: config.tournament.short_circuit_threshold,
            },
        })
    }
}

/// A directory level under construction: children in name order,
/// plus the mtime carried by an explicit `add_dir` (implicit
/// directories created by a nested file path keep mtime 0).
struct StreamDir {
    mtime_ns: u64,
    /// Permission bits for the emitted directory inode. The default
    /// (root and implicit parents) mirrors the directory writer's
    /// historical 0755 — a zero here would lock extracted trees.
    perms: u32,
    children: BTreeMap<String, StreamNode>,
}

impl Default for StreamDir {
    fn default() -> Self {
        Self {
            mtime_ns: 0,
            perms: 0o755,
            children: BTreeMap::new(),
        }
    }
}

enum StreamNode {
    Dir(StreamDir),
    /// Inode already pushed; the number wires the tree at `finish`.
    File {
        inode_number: u64,
    },
    Symlink {
        inode_number: u64,
    },
}

/// Build one `.lim` image from a sequence of named streams.
///
/// Create with [`StreamWriter::new`], add entries in any order
/// ([`add_file`], [`add_dir`], [`add_symlink`]), then [`finish`] to
/// assemble the artifact. Names are `/`-separated image-relative
/// paths; parent directories are materialised implicitly, or
/// explicitly via [`add_dir`] to control mtimes and empty
/// directories.
///
/// [`add_file`]: Self::add_file
/// [`add_dir`]: Self::add_dir
/// [`add_symlink`]: Self::add_symlink
/// [`finish`]: Self::finish
///
/// # Errors
///
/// [`WriteError::Io`] on reader failure, invalid names, conflicting
/// paths, or any writer-pipeline error.
pub struct StreamWriter<'a> {
    ctx: WriteContext,
    config: &'a WriteConfig,
    codecs: StreamCodecs,
    inline_threshold: u64,
    tree: StreamDir,
    /// Random-access entries awaiting the parallel flush at
    /// `finish` (see [`stage_file`]). Flushed in stage order, after
    /// every immediate `add_*` call.
    ///
    /// [`stage_file`]: Self::stage_file
    staged: Vec<StagedEntry<'a>>,
}

/// One deferred stream entry: the tree slot and inode exist; only
/// the chunk/hash/compress work is pending.
struct StagedEntry<'a> {
    name: String,
    mtime_ns: u64,
    perms: u32,
    xattrs: Vec<limnifs_core::inode::XAttr>,
    inode_number: u64,
    data: &'a [u8],
}

impl<'a> StreamWriter<'a> {
    /// Start a stream write under `config`.
    ///
    /// # Errors
    ///
    /// [`WriteError::Io`] if the config's codec registry or chunking
    /// parameters are invalid.
    pub fn new(config: &'a WriteConfig) -> Result<Self, WriteError> {
        let mut ctx = WriteContext::new();
        ctx.chunker = crate::chunker_from_config(config)?;
        ctx.categorizers_disabled = config.categorizers.is_empty();
        ctx.rw_mode = matches!(config.mode, crate::config::ImageMode::ReadWrite(_));
        ctx.auto_turnover = config.turnover_threshold > 0;
        ctx.collect_dict_samples = config.dictionaries.enabled;
        ctx.inline_threshold = config.defaults.inline_threshold as usize;
        ctx.metadata_externalize_threshold = config.defaults.metadata_externalize_threshold;
        ctx.emit_shared_inline = config.defaults.shared_inline;
        let classifier = ctx.classifier;
        let chunker = ctx.chunker.clone();
        Ok(Self {
            codecs: StreamCodecs::from_config(chunker, classifier, config)?,
            inline_threshold: u64::try_from(ctx.inline_threshold).unwrap_or(u64::MAX),
            ctx,
            config,
            tree: StreamDir::default(),
            staged: Vec::new(),
        })
    }

    /// Add a regular file at `name`, streaming `reader` through the
    /// chunker. Small entries (within the config's inline
    /// threshold) are stored inline, matching the directory writer.
    ///
    /// # Errors
    ///
    /// [`WriteError::Io`] if the name is invalid or conflicts with
    /// an existing entry, or the reader fails.
    pub fn add_file(
        &mut self,
        name: &str,
        mtime_ns: u64,
        perms: u32,
        xattrs: &[(String, Vec<u8>)],
        reader: &mut dyn Read,
    ) -> Result<(), WriteError> {
        let (parent, leaf) = descend(&mut self.tree, name)?;
        if parent.children.contains_key(leaf) {
            return Err(name_conflict(name));
        }
        let inode_number = self.ctx.alloc_inode();
        let pf = PendingFile {
            path: PathBuf::from(name),
            inode_number,
            file_len: 0,
            mtime_ns,
            mode: limnifs_core::inode::S_IFREG | (perms & 0o7777),
            uid: 0,
            gid: 0,
        };
        let wire_xattrs = to_wire_xattrs(xattrs)?;
        self.ctx.pending_files.push(pf.clone());

        let chunks = self.codecs.chunker.chunk_reader(reader)?;
        let total_len: u64 = chunks.iter().map(|c| c.len() as u64).sum();
        self.ctx.file_count += 1;

        if total_len <= self.inline_threshold {
            let mut data = Vec::with_capacity(total_len as usize);
            for chunk in &chunks {
                data.extend_from_slice(chunk);
            }
            self.ctx.inodes.push(PendingInode {
                number: inode_number,
                mode: limnifs_core::inode::S_IFREG | (perms & 0o7777),
                uid: 0,
                gid: 0,
                mtime_ns,
                xattrs: wire_xattrs,
                content: PendingContent::Inline(data),
            });
        } else {
            let mut drops = Vec::with_capacity(chunks.len());
            let mut slices = Vec::with_capacity(chunks.len());
            let mut offset: u64 = 0;
            for chunk in &chunks {
                let drop_id = hash_section(chunk);
                slices.push(crate::PendingSlice {
                    drop_id,
                    file_byte_start: offset,
                    file_byte_end: offset + chunk.len() as u64,
                });
                offset += chunk.len() as u64;
                let class = self.codecs.classifier.classify(chunk);
                let (codec_id, compressed) = crate::compress_chunk_with_tournament(
                    chunk,
                    class,
                    self.codecs.text_codec,
                    self.codecs.binary_codec,
                    &self.codecs.tunables,
                    &self.codecs.tournament,
                );
                drops.push((drop_id, chunk.clone(), compressed, codec_id, 0));
            }
            self.ctx
                .merge_chunked_file(&pf, crate::ChunkedFileResult { drops, slices });
            // merge_chunked_file read file_len from the placeholder;
            // correct the just-pushed inode now that it is known.
            if let Some(inode) = self.ctx.inodes.last_mut() {
                if let PendingContent::DropBacked { file_len, .. } = &mut inode.content {
                    *file_len = total_len;
                }
            }
            self.ctx
                .pending_files
                .last_mut()
                .expect("pushed above")
                .file_len = total_len;
        }
        parent
            .children
            .insert(leaf.to_owned(), StreamNode::File { inode_number });
        Ok(())
    }

    /// Stage a random-access entry for parallel packing: like
    /// [`add_file`], but the data is an in-memory slice (e.g. an
    /// mmap'd archive entry) whose byte range is already known, so
    /// chunk/hash/compress work is deferred to [`finish`], which
    /// fans the staged set across rayon workers and merges the
    /// results serially in stage order. Same entries, same order →
    /// byte-identical image to the serial `add_file` path.
    ///
    /// The borrow of `data` must outlive the writer.
    ///
    /// # Errors
    ///
    /// [`WriteError::Io`] if the name is invalid or conflicts with
    /// an existing entry.
    pub fn stage_file(
        &mut self,
        name: &str,
        mtime_ns: u64,
        perms: u32,
        xattrs: &[(String, Vec<u8>)],
        data: &'a [u8],
    ) -> Result<(), WriteError> {
        let (parent, leaf) = descend(&mut self.tree, name)?;
        if parent.children.contains_key(leaf) {
            return Err(name_conflict(name));
        }
        let inode_number = self.ctx.alloc_inode();
        self.ctx.file_count += 1;
        crate::progress::emit_file(std::path::Path::new(name), data.len() as u64);
        parent
            .children
            .insert(leaf.to_owned(), StreamNode::File { inode_number });
        self.staged.push(StagedEntry {
            name: name.to_owned(),
            mtime_ns,
            perms,
            xattrs: to_wire_xattrs(xattrs)?,
            inode_number,
            data,
        });
        Ok(())
    }

    /// Add (or declare) a directory at `name` with the given mtime.
    /// Implicit parents created by nested entries keep mtime 0;
    /// calling this on an existing implicit directory stamps it.
    ///
    /// # Errors
    ///
    /// [`WriteError::Io`] if the name is invalid or conflicts with
    /// a non-directory entry.
    pub fn add_dir(&mut self, name: &str, mtime_ns: u64, perms: u32) -> Result<(), WriteError> {
        if name == "/" {
            return Ok(()); // the root is materialised at finish
        }
        let (parent, leaf) = descend(&mut self.tree, name)?;
        match parent.children.get_mut(leaf) {
            None => {
                parent.children.insert(
                    leaf.to_owned(),
                    StreamNode::Dir(StreamDir {
                        mtime_ns,
                        perms,
                        children: BTreeMap::new(),
                    }),
                );
                Ok(())
            }
            Some(StreamNode::Dir(dir)) => {
                dir.mtime_ns = mtime_ns;
                Ok(())
            }
            Some(_) => Err(name_conflict(name)),
        }
    }

    /// Add a hardlink at `name` referencing the file already added
    /// (by any path method) at `target` — both names share one
    /// inode, and the emitted inode carries the real nlink. The
    /// target must exist in the tree and be a regular file.
    ///
    /// # Errors
    ///
    /// [`WriteError::Io`] if `name` is invalid or conflicting, the
    /// target is missing, or the target is not a regular file.
    pub fn add_hardlink(&mut self, name: &str, target: &str) -> Result<(), WriteError> {
        // Resolve the target before mutably borrowing the tree for
        // the new name.
        let inode_number = resolve_file_inode(&self.tree, target)?;
        let (parent, leaf) = descend(&mut self.tree, name)?;
        if parent.children.contains_key(leaf) {
            return Err(name_conflict(name));
        }
        *self.ctx.nlink_counts.entry(inode_number).or_insert(1) += 1;
        parent
            .children
            .insert(leaf.to_owned(), StreamNode::File { inode_number });
        Ok(())
    }

    /// Add a symbolic link at `name` pointing at `target` (stored
    /// raw, exactly as given).
    ///
    /// # Errors
    ///
    /// [`WriteError::Io`] if the name is invalid or conflicts with
    /// an existing entry.
    pub fn add_symlink(
        &mut self,
        name: &str,
        target: &str,
        mtime_ns: u64,
        perms: u32,
    ) -> Result<(), WriteError> {
        let (parent, leaf) = descend(&mut self.tree, name)?;
        if parent.children.contains_key(leaf) {
            return Err(name_conflict(name));
        }
        let inode_number = self.ctx.alloc_inode();
        self.ctx.inodes.push(PendingInode {
            number: inode_number,
            mode: limnifs_core::inode::S_IFLNK | (perms & 0o7777),
            uid: 0,
            gid: 0,
            mtime_ns,
            xattrs: Vec::new(),
            content: PendingContent::Symlink(target.to_owned()),
        });
        parent
            .children
            .insert(leaf.to_owned(), StreamNode::Symlink { inode_number });
        Ok(())
    }

    /// Materialise the tree and assemble the image.
    ///
    /// # Errors
    ///
    /// [`WriteError::Io`] on any writer-pipeline error.
    pub fn finish(mut self) -> Result<WriteArtifact, WriteError> {
        self.flush_staged()?;
        let tree = std::mem::take(&mut self.tree);
        self.ctx.root_inode_number = self.materialize_dir(tree);
        self.ctx
            .train_and_apply_dictionary(&self.config.dictionaries);
        Ok(self.ctx.assemble())
    }

    /// Chunk/hash/compress every staged entry across rayon workers,
    /// then merge serially in stage order. The parallel map is
    /// order-preserving and the merge replays the exact same
    /// per-entry steps as [`add_file`], so output is identical to
    /// the serial path. Large entries additionally hit the
    /// boundary-identical parallel FastCDC inside their slice —
    /// nested rayon, the same work-stealing shape the write
    /// pipeline already uses.
    fn flush_staged(&mut self) -> Result<(), WriteError> {
        if self.staged.is_empty() {
            return Ok(());
        }
        let staged = std::mem::take(&mut self.staged);
        let codecs = &self.codecs;
        use rayon::prelude::*;
        let results: Vec<crate::ChunkedFileResult> = staged
            .par_iter()
            .map(|entry| {
                let chunks: Vec<&[u8]> = codecs.chunker.chunk_slice(entry.data);
                let mut drops = Vec::with_capacity(chunks.len());
                let mut slices = Vec::with_capacity(chunks.len());
                let mut offset: u64 = 0;
                for chunk in &chunks {
                    let drop_id = crate::hash_section(chunk);
                    slices.push(crate::PendingSlice {
                        drop_id,
                        file_byte_start: offset,
                        file_byte_end: offset + chunk.len() as u64,
                    });
                    offset += chunk.len() as u64;
                    let class = codecs.classifier.classify(chunk);
                    let (codec_id, compressed) = crate::compress_chunk_with_tournament(
                        chunk,
                        class,
                        codecs.text_codec,
                        codecs.binary_codec,
                        &codecs.tunables,
                        &codecs.tournament,
                    );
                    drops.push((drop_id, (*chunk).to_vec(), compressed, codec_id, 0));
                }
                crate::ChunkedFileResult { drops, slices }
            })
            .collect();
        for (entry, result) in staged.iter().zip(results) {
            // Unlike the streaming path, the length is known upfront,
            // so no post-merge inode patching is needed.
            let total_len = entry.data.len() as u64;
            let pf = PendingFile {
                path: std::path::PathBuf::from(&entry.name),
                inode_number: entry.inode_number,
                file_len: total_len,
                mtime_ns: entry.mtime_ns,
                mode: limnifs_core::inode::S_IFREG | (entry.perms & 0o7777),
                uid: 0,
                gid: 0,
            };
            self.ctx.pending_files.push(pf.clone());
            if total_len <= self.inline_threshold {
                // Below the inline threshold chunk_slice yields the
                // whole entry as one chunk, so this equals the
                // serial path's chunk concatenation.
                let mut data = Vec::with_capacity(entry.data.len());
                data.extend_from_slice(entry.data);
                self.ctx.inodes.push(PendingInode {
                    number: entry.inode_number,
                    mode: limnifs_core::inode::S_IFREG | (entry.perms & 0o7777),
                    uid: 0,
                    gid: 0,
                    mtime_ns: entry.mtime_ns,
                    xattrs: entry.xattrs.clone(),
                    content: PendingContent::Inline(data),
                });
            } else {
                if !entry.xattrs.is_empty() {
                    self.ctx
                        .inode_xattrs
                        .insert(entry.inode_number, entry.xattrs.clone());
                }
                self.ctx.merge_chunked_file(&pf, result);
            }
        }
        Ok(())
    }

    /// Allocate this directory's inode, then recurse into children
    /// in name order — parent-first, mirroring the directory walk.
    fn materialize_dir(&mut self, dir: StreamDir) -> u64 {
        let inode_number = self.ctx.alloc_inode();
        self.ctx.dir_count += 1;
        let mut entries = Vec::with_capacity(dir.children.len());
        for (name, node) in dir.children {
            let (child_inode, entry_type) = match node {
                StreamNode::Dir(child) => (self.materialize_dir(child), 0x02),
                StreamNode::File { inode_number } => (inode_number, 0x01),
                StreamNode::Symlink { inode_number } => (inode_number, 0x03),
            };
            entries.push((name, child_inode, entry_type));
        }
        // BTreeMap iterates name-sorted; the explicit sort keeps the
        // invariant local, exactly like fold_survey.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        self.ctx.dir_nodes.push(encode_dir_node(&entries));
        self.ctx.inodes.push(PendingInode {
            number: inode_number,
            mode: limnifs_core::inode::S_IFDIR | (dir.perms & 0o7777),
            uid: 0,
            gid: 0,
            mtime_ns: dir.mtime_ns,
            xattrs: Vec::new(),
            content: PendingContent::Directory(entries),
        });
        inode_number
    }
}

/// Walk (creating implicit directories) to `name`'s parent and
/// return it plus the leaf component.
fn descend<'a, 'b>(
    root: &'a mut StreamDir,
    name: &'b str,
) -> Result<(&'a mut StreamDir, &'b str), WriteError> {
    if name.is_empty() || name.starts_with('/') || name.ends_with('/') {
        return Err(bad_name(name));
    }
    let mut dir = root;
    let mut components = name.split('/').peekable();
    let leaf = components.next_back().expect("non-empty name has a leaf");
    for component in components {
        if component.is_empty() || component == "." || component == ".." {
            return Err(bad_name(name));
        }
        dir = match dir
            .children
            .entry(component.to_owned())
            .or_insert_with(|| StreamNode::Dir(StreamDir::default()))
        {
            StreamNode::Dir(child) => child,
            StreamNode::File { .. } | StreamNode::Symlink { .. } => {
                return Err(name_conflict(name))
            }
        };
    }
    if leaf.is_empty() || leaf == "." || leaf == ".." {
        return Err(bad_name(name));
    }
    Ok((dir, leaf))
}

/// Resolve `target` to a file entry's inode number inside the
/// stream tree. Errors when any component is missing, is a
/// symlink, or the final component is a directory.
fn resolve_file_inode(root: &StreamDir, target: &str) -> Result<u64, WriteError> {
    let bad = || {
        WriteError::Io(std::io::Error::other(format!(
            "hardlink target {target:?} is not a file in the tree"
        )))
    };
    let mut dir = root;
    let mut components = target.split('/').filter(|c| !c.is_empty());
    loop {
        let Some(component) = components.next() else {
            return Err(bad());
        };
        match dir.children.get(component) {
            Some(StreamNode::File { inode_number }) if components.next().is_none() => {
                return Ok(*inode_number);
            }
            Some(StreamNode::Dir(child)) => dir = child,
            _ => return Err(bad()),
        }
    }
}

/// Validate and convert caller-supplied xattrs to wire form:
/// namespace 0, 64 KiB total cap (metadata DoS guard), and the pax
/// transport's hard limits — keys and values must be NUL-free and
/// newline-free (a pax record is a length-prefixed text line).
/// Returns an error naming the offending attribute.
fn to_wire_xattrs(
    raw: &[(String, Vec<u8>)],
) -> Result<Vec<limnifs_core::inode::XAttr>, WriteError> {
    const TOTAL_CAP: usize = 64 * 1024;
    let mut out = Vec::with_capacity(raw.len());
    let mut total = 0usize;
    for (key, value) in raw {
        if key.contains('\0') || key.contains('\n') || value.contains(&0) || value.contains(&b'\n')
        {
            return Err(WriteError::Io(std::io::Error::other(format!(
                "xattr {key:?} carries NUL or newline bytes the pax record format cannot represent"
            ))));
        }
        total += key.len() + value.len();
        if total > TOTAL_CAP {
            break;
        }
        out.push(limnifs_core::inode::XAttr {
            namespace: 0,
            key: key.clone(),
            value: value.clone(),
        });
    }
    Ok(out)
}

fn bad_name(name: &str) -> WriteError {
    WriteError::Io(std::io::Error::other(format!(
        "invalid stream entry name {name:?}: must be a non-empty relative path without '.' or '..' components"
    )))
}

fn name_conflict(name: &str) -> WriteError {
    WriteError::Io(std::io::Error::other(format!(
        "stream entry conflict: {name:?} already exists with a different type"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn writer() -> StreamWriter<'static> {
        // Leak is test-only and the config has no drop significance.
        let config: &'static WriteConfig = Box::leak(Box::new(WriteConfig::default_v0_1()));
        StreamWriter::new(config).expect("default config is valid")
    }

    fn pseudo_random_bytes(seed: u64, count: usize) -> Vec<u8> {
        let mut state = seed;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            out.push(u8::try_from(state >> 56).expect("fits u8"));
        }
        out
    }

    fn add_all(w: &mut StreamWriter<'_>) {
        w.add_dir("docs", 7_000_000_000_000, 0o755).expect("dir");
        w.add_file(
            "docs/readme.txt",
            1_000_000_000,
            0o644,
            &[],
            &mut b"hello stream writer\n".as_slice(),
        )
        .expect("file 1");
        let big = pseudo_random_bytes(9, 600 * 1024);
        w.add_file(
            "data/big.bin",
            2_000_000_000,
            0o755,
            &[],
            &mut big.as_slice(),
        )
        .expect("file 2");
        w.add_symlink("latest", "docs/readme.txt", 3_000_000_000, 0o777)
            .expect("symlink");
    }

    #[test]
    fn same_entry_sequence_packs_identically() {
        let a = {
            let mut w = writer();
            add_all(&mut w);
            w.finish().expect("finish a").bytes
        };
        let b = {
            let mut w = writer();
            add_all(&mut w);
            w.finish().expect("finish b").bytes
        };
        assert_eq!(a, b);
    }

    #[test]
    fn empty_stream_writes_root_only() {
        let artifact = writer().finish().expect("finish");
        assert_eq!(artifact.dir_count, 1);
        assert_eq!(artifact.file_count, 0);
        assert!(artifact.slabs.is_empty());
    }

    #[test]
    fn small_files_inline_and_big_files_slab() {
        let artifact = {
            let mut w = writer();
            add_all(&mut w);
            w.finish().expect("finish")
        };
        assert_eq!(artifact.file_count, 2);
        assert_eq!(artifact.dir_count, 3); // root + docs + data (implicit)
        assert_eq!(artifact.slabs.len(), 1);
    }

    #[test]
    fn staged_path_is_byte_identical_to_serial() {
        let big_a = pseudo_random_bytes(31, 600 * 1024);
        let big_b = pseudo_random_bytes(32, 900 * 1024);
        let staged = {
            let mut w = writer();
            w.add_dir("docs", 7_000_000_000_000, 0o755).expect("dir");
            w.add_file(
                "tiny.txt",
                1,
                0o644,
                &[],
                &mut b"small inline entry\n".as_slice(),
            )
            .expect("immediate file");
            w.stage_file("docs/a.bin", 2, 0o644, &[], &big_a)
                .expect("staged a");
            w.stage_file("docs/b.bin", 3, 0o755, &[], &big_b)
                .expect("staged b");
            w.stage_file("docs/tiny2.txt", 4, 0o600, &[], b"also inline\n")
                .expect("staged tiny");
            w.finish().expect("finish staged").bytes
        };
        let serial = {
            let mut w = writer();
            w.add_dir("docs", 7_000_000_000_000, 0o755).expect("dir");
            w.add_file(
                "tiny.txt",
                1,
                0o644,
                &[],
                &mut b"small inline entry\n".as_slice(),
            )
            .expect("immediate file");
            w.add_file("docs/a.bin", 2, 0o644, &[], &mut big_a.as_slice())
                .expect("serial a");
            w.add_file("docs/b.bin", 3, 0o755, &[], &mut big_b.as_slice())
                .expect("serial b");
            w.add_file(
                "docs/tiny2.txt",
                4,
                0o600,
                &[],
                &mut b"also inline\n".as_slice(),
            )
            .expect("serial tiny");
            w.finish().expect("finish serial").bytes
        };
        assert_eq!(staged, serial, "staged flush must equal the serial path");
    }

    #[test]
    fn staged_detects_conflicts_and_bad_names() {
        let mut w = writer();
        w.stage_file("a.txt", 0, 0o644, &[], b"x").expect("stage");
        assert!(w.stage_file("a.txt", 0, 0o644, &[], b"y").is_err());
        assert!(w.stage_file("", 0, 0o644, &[], b"y").is_err());
        assert!(w.stage_file("/abs", 0, 0o644, &[], b"y").is_err());
        assert!(w.stage_file("a.txt/child", 0, 0o644, &[], b"y").is_err());
    }

    #[test]
    fn rejects_bad_and_conflicting_names() {
        let mut w = writer();
        assert!(w.add_file("", 0, 0o644, &[], &mut [].as_slice()).is_err());
        assert!(w
            .add_file("/abs", 0, 0o644, &[], &mut [].as_slice())
            .is_err());
        assert!(w
            .add_file("a/../b", 0, 0o644, &[], &mut [].as_slice())
            .is_err());
        assert!(w
            .add_file("ok.txt", 0, 0o644, &[], &mut [].as_slice())
            .is_ok());
        // Same leaf again, even with identical type: conflict.
        assert!(w
            .add_file("ok.txt", 0, 0o644, &[], &mut [].as_slice())
            .is_err());
        // File where a directory must pass through.
        assert!(w
            .add_file("ok.txt/child", 0, 0o644, &[], &mut [].as_slice())
            .is_err());
        // Symlink over a file.
        assert!(w.add_symlink("ok.txt", "x", 0, 0o777).is_err());
    }
}
