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
#[derive(Default)]
struct StreamDir {
    mtime_ns: u64,
    children: BTreeMap<String, StreamNode>,
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
        };
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
                mode: 0o100_644,
                mtime_ns,
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

    /// Add (or declare) a directory at `name` with the given mtime.
    /// Implicit parents created by nested entries keep mtime 0;
    /// calling this on an existing implicit directory stamps it.
    ///
    /// # Errors
    ///
    /// [`WriteError::Io`] if the name is invalid or conflicts with
    /// a non-directory entry.
    pub fn add_dir(&mut self, name: &str, mtime_ns: u64) -> Result<(), WriteError> {
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
    ) -> Result<(), WriteError> {
        let (parent, leaf) = descend(&mut self.tree, name)?;
        if parent.children.contains_key(leaf) {
            return Err(name_conflict(name));
        }
        let inode_number = self.ctx.alloc_inode();
        self.ctx.inodes.push(PendingInode {
            number: inode_number,
            mode: limnifs_core::inode::S_IFLNK | 0o777,
            mtime_ns,
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
        let tree = std::mem::take(&mut self.tree);
        self.ctx.root_inode_number = self.materialize_dir(tree);
        self.ctx
            .train_and_apply_dictionary(&self.config.dictionaries);
        Ok(self.ctx.assemble())
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
            mode: 0o040_755,
            mtime_ns: dir.mtime_ns,
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
        w.add_dir("docs", 7_000_000_000_000).expect("dir");
        w.add_file(
            "docs/readme.txt",
            1_000_000_000,
            &mut b"hello stream writer\n".as_slice(),
        )
        .expect("file 1");
        let big = pseudo_random_bytes(9, 600 * 1024);
        w.add_file("data/big.bin", 2_000_000_000, &mut big.as_slice())
            .expect("file 2");
        w.add_symlink("latest", "docs/readme.txt", 3_000_000_000)
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
    fn rejects_bad_and_conflicting_names() {
        let mut w = writer();
        assert!(w.add_file("", 0, &mut [].as_slice()).is_err());
        assert!(w.add_file("/abs", 0, &mut [].as_slice()).is_err());
        assert!(w.add_file("a/../b", 0, &mut [].as_slice()).is_err());
        assert!(w.add_file("ok.txt", 0, &mut [].as_slice()).is_ok());
        // Same leaf again, even with identical type: conflict.
        assert!(w.add_file("ok.txt", 0, &mut [].as_slice()).is_err());
        // File where a directory must pass through.
        assert!(w.add_file("ok.txt/child", 0, &mut [].as_slice()).is_err());
        // Symlink over a file.
        assert!(w.add_symlink("ok.txt", "x", 0).is_err());
    }
}
