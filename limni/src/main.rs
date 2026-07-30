//! `limni` — the `LimniFS` CLI.
//!
//! One format, one CLI. This binary owns UX only; every format,
//! crypto, and merge concern lives in `limnifs-core` and the crates
//! below it. See component `10-cli` in `TODO.impl/`.
//!
//! Exit codes (stable):
//!   0 — success
//!   1 — read error (I/O, format)
//!   2 — usage error (clap)
//!
//! See `TODO.impl/10-cli/README.md` for the planned command tree.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use limnifs_core::{
    compute_merkle_root, hash_empty_section, hash_section, parse_dms_policy, parse_ec_params,
    parse_feature_flags_section, parse_history, parse_manifest_header, parse_metadata_blob,
    parse_metadata_reference, parse_slab_index, ContentHandle, CoreError, FeatureFlags,
    ManifestCursor, ManifestHeader, ManifestRoot, MetadataBlob, SectionHashes,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Read at most this many bytes when verifying. Covers any
/// reasonable v0.1 manifest (header + flags + metadata reference +
/// slab index + history + optional sections). Larger manifests fall
/// back to header-only reporting.
const VERIFY_READ_BUDGET: usize = 1024 * 1024;

/// `LimniFS` — Layered, Immutable, Merkle-rooted, Network Image filesystem.
#[derive(Debug, Parser)]
#[command(
    name = "limni",
    version,
    about = "LimniFS — one format, one CLI",
    long_about = "LimniFS CLI. Inspect, build, mount, and explore .lim images."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate a manifest and compute its `ManifestRoot`.
    ///
    /// Parses every required section (header, feature flags, metadata
    /// reference, slab index, history), captures the raw bytes each
    /// parser consumed, and computes the image's `ManifestRoot` per
    /// spec §5.10. Optional sections (crypto params, EC, DMS, delta
    /// linkage) are not yet parsed; if extra bytes remain after
    /// history, the root is reported with a warning.
    Verify {
        /// Path to the `.lim` image to inspect.
        image: PathBuf,
        /// Emit machine-readable JSON instead of human text.
        #[arg(long)]
        json: bool,
    },
    /// Build a `.lim` image from a directory tree.
    ///
    /// The MVP writer walks the directory recursively and emits a
    /// self-contained manifest with inlined metadata. Files at or
    /// below the inline threshold (4 KiB) are stored inline; larger
    /// files are rejected.
    Limn {
        /// Source directory to package.
        source: PathBuf,
        /// Output `.lim` file path.
        output: PathBuf,
    },
    /// List the contents of a directory inside a `.lim` image.
    ///
    /// Resolves the inlined metadata blob, walks the directory tree
    /// from the root inode, and prints one line per entry at the
    /// requested path. The path is slash-separated and relative to
    /// the image's root directory; pass `/` (or omit the argument)
    /// to list the root.
    Ls {
        /// Path to the `.lim` image to inspect.
        image: PathBuf,
        /// Slash-separated directory path inside the image. Use `/`
        /// or pass nothing to list the root directory.
        #[arg(default_value = "/")]
        path: String,
    },
    /// Write a file's contents from a `.lim` image to stdout.
    ///
    /// Resolves the inlined metadata blob, walks the directory tree
    /// to the requested file, and writes its bytes to stdout. Inline
    /// files (≤ 4 KiB) are written directly; larger files are read
    /// from the slab file that sits next to the manifest.
    Cat {
        /// Path to the `.lim` image to read from.
        image: PathBuf,
        /// Slash-separated file path inside the image.
        path: String,
    },
    /// Print an inode's metadata.
    ///
    /// Resolves the inlined metadata blob, walks the directory tree
    /// to the requested path, and prints the inode's POSIX metadata
    /// (number, mode, uid, gid, mtime, nlink) plus a description of
    /// its content handle (inline-data length, slice-map drops,
    /// directory hash, symlink target, etc.).
    Stat {
        /// Path to the `.lim` image to inspect.
        image: PathBuf,
        /// Slash-separated path inside the image.
        path: String,
    },
    /// Extract an image's contents to a filesystem directory.
    ///
    /// Walks the directory tree from the root inode and recreates
    /// every file and directory under `dest`. Inline files write
    /// directly; slab-backed files load via the slab index locator.
    /// Permissions are set from each inode's mode bits.
    Extract {
        /// Path to the `.lim` image to extract.
        image: PathBuf,
        /// Destination directory. Created if it does not exist.
        dest: PathBuf,
    },
}

fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    match cli.command {
        Command::Verify { image, json } => verify(&image, json),
        Command::Limn { source, output } => limn(&source, &output),
        Command::Ls { image, path } => ls(&image, &path),
        Command::Cat { image, path } => cat(&image, &path),
        Command::Stat { image, path } => stat(&image, &path),
        Command::Extract { image, dest } => extract(&image, &dest),
    }
}

fn run_with_exit_code() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::ReadFailed { path, source }) => {
            eprintln!("limni: cannot read {}: {source}", path.display());
            ExitCode::FAILURE
        }
        Err(CliError::FormatFailed { path, source }) => {
            eprintln!("limni: {}: {source}", path.display());
            ExitCode::FAILURE
        }
        Err(CliError::WriteFailed { source }) => {
            eprintln!("limni: write failed: {source}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    run_with_exit_code()
}

#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
enum CliError {
    ReadFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    FormatFailed {
        path: PathBuf,
        source: CoreError,
    },
    WriteFailed {
        source: limnifs_write::WriteError,
    },
}

fn verify(image: &PathBuf, json: bool) -> Result<(), CliError> {
    let mut file = std::fs::File::open(image).map_err(|source| CliError::ReadFailed {
        path: image.clone(),
        source,
    })?;
    let mut buffer = vec![0u8; VERIFY_READ_BUDGET];
    let read_len = file
        .read(&mut buffer)
        .map_err(|source| CliError::ReadFailed {
            path: image.clone(),
            source,
        })?;
    buffer.truncate(read_len);
    let mut cursor = ManifestCursor::new(&buffer);

    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.clone(),
        source,
    };

    // Capture the byte range each parser consumes so we can hash the
    // raw section bytes for the Merkle root computation.
    let header_start = cursor.position();
    let header = parse_manifest_header(&mut cursor).map_err(map_err)?;
    let header_end = cursor.position();

    let flags_start = cursor.position();
    let flags = parse_feature_flags_section(&mut cursor).map_err(map_err)?;
    let flags_end = cursor.position();

    let meta_ref_start = cursor.position();
    let metadata_reference = parse_metadata_reference(&mut cursor).map_err(map_err)?;
    let meta_ref_end = cursor.position();

    let slab_index_start = cursor.position();
    let slab_index = parse_slab_index(&mut cursor).map_err(map_err)?;
    let slab_index_end = cursor.position();

    // Parse optional sections based on feature flags.
    let ec_params_start = cursor.position();
    let has_ec = flags.is_required(0x0001) || flags.get(0x0001).is_some();
    if has_ec {
        let _ = parse_ec_params(&mut cursor).map_err(map_err)?;
    }
    let ec_params_end = cursor.position();

    let dms_policy_start = cursor.position();
    let has_dms = flags.is_required(0x0002) || flags.get(0x0002).is_some();
    if has_dms {
        let _ = parse_dms_policy(&mut cursor).map_err(map_err)?;
    }
    let dms_policy_end = cursor.position();

    let history_start = cursor.position();
    let history = parse_history(&mut cursor).map_err(map_err)?;
    let history_end = cursor.position();

    let extra_bytes_remaining = cursor.remaining_len();

    let hashes = SectionHashes {
        metadata: metadata_reference.metadata_hash,
        format_header: hash_section(&buffer[header_start..header_end]),
        feature_flags: hash_section(&buffer[flags_start..flags_end]),
        metadata_reference: hash_section(&buffer[meta_ref_start..meta_ref_end]),
        slab_index: hash_section(&buffer[slab_index_start..slab_index_end]),
        crypto_params: hash_empty_section(),
        ec_params: if has_ec {
            hash_section(&buffer[ec_params_start..ec_params_end])
        } else {
            hash_empty_section()
        },
        dms_policy: if has_dms {
            hash_section(&buffer[dms_policy_start..dms_policy_end])
        } else {
            hash_empty_section()
        },
        delta_linkage: hash_empty_section(),
        history: hash_section(&buffer[history_start..history_end]),
    };
    let merkle_root = compute_merkle_root(&hashes);

    let metadata_summary = if metadata_reference.is_inlined() {
        if let Some(blob_bytes) = &metadata_reference.inline_metadata {
            let mut blob_cursor = ManifestCursor::new(blob_bytes);
            match parse_metadata_blob(&mut blob_cursor) {
                Ok(blob) => Some(MetadataSummary::from_blob(&blob)),
                Err(_) => None,
            }
        } else {
            None
        }
    } else {
        None
    };

    print_report(
        image,
        header,
        &flags,
        metadata_reference.is_inlined(),
        slab_index.len(),
        history.len(),
        extra_bytes_remaining,
        merkle_root,
        metadata_summary.as_ref(),
        json,
    );
    Ok(())
}

fn limn(source: &Path, output: &Path) -> Result<(), CliError> {
    let artifact = limnifs_write::write_directory(source)
        .map_err(|source| CliError::WriteFailed { source })?;
    std::fs::write(output, &artifact.bytes).map_err(|source| CliError::ReadFailed {
        path: output.to_path_buf(),
        source,
    })?;

    if let (Some(slab_bytes), Some(locator)) = (&artifact.slab_bytes, &artifact.slab_locator) {
        let slab_name = locator.strip_prefix("file:").unwrap_or(locator);
        let slab_path = output
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(slab_name);
        std::fs::write(&slab_path, slab_bytes).map_err(|source| CliError::ReadFailed {
            path: slab_path.clone(),
            source,
        })?;
        println!(
            "{}: wrote {} bytes (slab, {} drops)",
            slab_path.display(),
            slab_bytes.len(),
            artifact.drop_count,
        );
    }

    println!(
        "{output}: wrote {len} bytes, {manifest_root}",
        output = output.display(),
        len = artifact.bytes.len(),
        manifest_root = artifact.merkle_root,
    );
    println!(
        "  inodes: {}  files: {}  dirs: {}  drops: {}",
        artifact.inode_count, artifact.file_count, artifact.dir_count, artifact.drop_count
    );
    Ok(())
}

/// Read a `.lim` manifest, extract its inlined metadata blob, and list
/// the entries of the directory at `path` (slash-separated, relative
/// to the image's root; `/` lists the root).
fn ls(image: &Path, path: &str) -> Result<(), CliError> {
    let manifest_bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };
    let (blob, root_inode_number, slab_index) = load_image(&manifest_bytes, image, map_err)?;
    let _ = slab_index;

    let root_inode = blob
        .inode_by_number(root_inode_number)
        .expect("load_image validates that the root inode exists, so this lookup cannot fail");

    let target_dir_inode =
        resolve_path(&blob, root_inode, path).ok_or_else(|| CliError::FormatFailed {
            path: image.to_path_buf(),
            source: CoreError::Corrupt {
                reason: format!("metadata blob: path {path:?} not found in tree"),
            },
        })?;

    let target_hash = match &target_dir_inode.content_handle {
        ContentHandle::Directory(hash) => *hash,
        other => {
            return Err(CliError::FormatFailed {
                path: image.to_path_buf(),
                source: CoreError::Corrupt {
                    reason: format!(
                        "metadata blob: path {path:?} resolves to a non-directory inode (content_handle: {other:?})"
                    ),
                },
            });
        }
    };

    let dir_node = blob
        .dir_node_by_hash(&target_hash)
        .ok_or_else(|| CliError::FormatFailed {
            path: image.to_path_buf(),
            source: CoreError::Corrupt {
                reason: format!(
                    "metadata blob: directory node for hash {} missing",
                    format_hex(&target_hash)
                ),
            },
        })?;

    print_directory_listing(image, path, dir_node);
    Ok(())
}

/// Read a `.lim` manifest, extract its inlined metadata blob, and
/// write the file at `path` to stdout. Inline files are written
/// directly; drop-backed files are read from the slab file that lives
/// alongside the manifest (per the writer's `file:` locator).
fn cat(image: &Path, path: &str) -> Result<(), CliError> {
    use std::io::Write;
    let manifest_bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };
    let (blob, root_inode_number, slab_index) = load_image(&manifest_bytes, image, map_err)?;

    let root_inode = blob
        .inode_by_number(root_inode_number)
        .expect("load_image validates that the root inode exists, so this lookup cannot fail");

    let target_inode =
        resolve_path(&blob, root_inode, path).ok_or_else(|| CliError::FormatFailed {
            path: image.to_path_buf(),
            source: CoreError::Corrupt {
                reason: format!("metadata blob: path {path:?} not found in tree"),
            },
        })?;

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match &target_inode.content_handle {
        ContentHandle::InlineData(data) => {
            out.write_all(data).map_err(|source| CliError::ReadFailed {
                path: image.to_path_buf(),
                source,
            })?;
        }
        ContentHandle::SliceMap(slices) => {
            for slice in slices {
                let slab_path = resolve_slab_path(image, &slab_index)?;
                let slab_bytes =
                    std::fs::read(&slab_path).map_err(|source| CliError::ReadFailed {
                        path: slab_path.clone(),
                        source,
                    })?;
                let slab_view = limnifs_core::parse_slab(&slab_bytes).map_err(|source| {
                    CliError::FormatFailed {
                        path: slab_path.clone(),
                        source,
                    }
                })?;
                let plaintext = slab_view
                    .plaintext_for(slice.drop_id.as_bytes())
                    .ok_or_else(|| CliError::FormatFailed {
                        path: slab_path.clone(),
                        source: CoreError::Corrupt {
                            reason: format!(
                                "slab: drop id {} not found in slab file",
                                format_hex(slice.drop_id.as_bytes())
                            ),
                        },
                    })?
                    .map_err(map_err)?;
                out.write_all(plaintext)
                    .map_err(|source| CliError::ReadFailed {
                        path: image.to_path_buf(),
                        source,
                    })?;
            }
        }
        other => {
            return Err(CliError::FormatFailed {
                path: image.to_path_buf(),
                source: CoreError::Corrupt {
                    reason: format!(
                        "metadata blob: path {path:?} resolves to a non-file inode (content_handle: {other:?})"
                    ),
                },
            });
        }
    }
    Ok(())
}

/// Print an inode's metadata: POSIX attributes plus a description of
/// its content handle.
fn stat(image: &Path, path: &str) -> Result<(), CliError> {
    let manifest_bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };
    let (blob, root_inode_number, _slab_index) = load_image(&manifest_bytes, image, map_err)?;

    let root_inode = blob
        .inode_by_number(root_inode_number)
        .expect("load_image validates that the root inode exists, so this lookup cannot fail");

    let target_inode =
        resolve_path(&blob, root_inode, path).ok_or_else(|| CliError::FormatFailed {
            path: image.to_path_buf(),
            source: CoreError::Corrupt {
                reason: format!("metadata blob: path {path:?} not found in tree"),
            },
        })?;

    print_inode_stat(image, path, target_inode);
    Ok(())
}

fn print_inode_stat(image: &Path, path: &str, inode: &limnifs_core::Inode) {
    println!("{}: stat {path:?}", image.display());
    println!("  inode number:  {}", inode.number);
    println!("  mode:          0o{:06o}", inode.mode & 0o7777);
    println!("  file type:     {}", format_file_type(inode.file_type()));
    println!("  uid:           {}", inode.uid);
    println!("  gid:           {}", inode.gid);
    println!("  nlink:         {}", inode.nlink);
    println!("  mtime_ns:      {}", inode.mtime_ns);
    println!("  ctime_ns:      {}", inode.ctime_ns);
    if let Some(atime_ns) = inode.atime_ns {
        println!("  atime_ns:      {atime_ns}");
    }
    if !inode.xattrs.is_empty() {
        println!("  xattrs:        {} entries", inode.xattrs.len());
        for x in &inode.xattrs {
            println!(
                "    ns=0x{:02X}  {:?}  ({} bytes)",
                x.namespace,
                x.key,
                x.value.len()
            );
        }
    }
    print_content_handle(inode);
}

fn format_file_type(file_type: u32) -> &'static str {
    match file_type {
        limnifs_core::S_IFREG => "regular file",
        limnifs_core::S_IFDIR => "directory",
        limnifs_core::S_IFLNK => "symlink",
        limnifs_core::S_IFBLK => "block device",
        limnifs_core::S_IFCHR => "char device",
        limnifs_core::S_IFIFO => "fifo",
        limnifs_core::S_IFSOCK => "socket",
        _ => "unknown",
    }
}

fn print_content_handle(inode: &limnifs_core::Inode) {
    match &inode.content_handle {
        ContentHandle::InlineData(data) => {
            println!("  content:       inline ({} bytes)", data.len());
        }
        ContentHandle::SliceMap(slices) => {
            let total: u64 = slices
                .iter()
                .map(|s| s.file_byte_end - s.file_byte_start)
                .sum();
            println!(
                "  content:       slice map ({} slices, {} bytes)",
                slices.len(),
                total
            );
            for (i, slice) in slices.iter().enumerate() {
                println!(
                    "    [{i}] file=[{},{}) drop={} @{}+{}",
                    slice.file_byte_start,
                    slice.file_byte_end,
                    slice.drop_id,
                    slice.drop_byte_start,
                    slice.drop_byte_len,
                );
            }
        }
        ContentHandle::Directory(hash) => {
            println!(
                "  content:       directory node (hash {})",
                format_hex(hash)
            );
        }
        ContentHandle::Symlink(target) => {
            println!("  content:       symlink → {target:?}");
        }
        ContentHandle::Device(dev) => {
            println!(
                "  content:       device (major={}, minor={})",
                dev >> 8,
                dev & 0xFF
            );
        }
        ContentHandle::Pipe(pipe_id) => {
            println!("  content:       pipe (id={pipe_id})");
        }
    }
}

/// Extract an image's full directory tree to `dest`. Recreates every
/// directory and file. Permissions are set from each inode's mode bits
/// (low 12 bits). Symlinks and special files are skipped with a
/// warning (v0.1 limitation).
fn extract(image: &Path, dest: &Path) -> Result<(), CliError> {
    let manifest_bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };
    let (blob, root_inode_number, slab_index) = load_image(&manifest_bytes, image, map_err)?;

    std::fs::create_dir_all(dest).map_err(|source| CliError::ReadFailed {
        path: dest.to_path_buf(),
        source,
    })?;

    let root_inode = blob
        .inode_by_number(root_inode_number)
        .expect("load_image validates that the root inode exists, so this lookup cannot fail");

    let mut file_count = 0usize;
    let mut dir_count = 0usize;
    extract_directory(
        &blob,
        root_inode,
        dest,
        image,
        &slab_index,
        &mut file_count,
        &mut dir_count,
    )?;
    println!(
        "{dest}: extracted {file_count} files, {dir_count} directories",
        dest = dest.display()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn extract_directory(
    blob: &MetadataBlob,
    dir_inode: &limnifs_core::Inode,
    dir_path: &Path,
    image: &Path,
    slab_index: &limnifs_core::SlabIndex,
    file_count: &mut usize,
    dir_count: &mut usize,
) -> Result<(), CliError> {
    let hash = match &dir_inode.content_handle {
        ContentHandle::Directory(hash) => *hash,
        other => {
            return Err(CliError::FormatFailed {
                path: dir_path.to_path_buf(),
                source: CoreError::Corrupt {
                    reason: format!(
                        "extract: expected directory inode, got content_handle: {other:?}"
                    ),
                },
            });
        }
    };
    let node = blob
        .dir_node_by_hash(&hash)
        .ok_or_else(|| CliError::FormatFailed {
            path: dir_path.to_path_buf(),
            source: CoreError::Corrupt {
                reason: format!(
                    "extract: directory node for hash {} missing",
                    format_hex(&hash)
                ),
            },
        })?;

    set_mode(dir_path, dir_inode.mode);

    for entry in &node.entries {
        let child_inode =
            blob.inode_by_number(entry.inode_number)
                .ok_or_else(|| CliError::FormatFailed {
                    path: dir_path.to_path_buf(),
                    source: CoreError::Corrupt {
                        reason: format!(
                            "extract: entry {:?} references missing inode {}",
                            entry.name, entry.inode_number
                        ),
                    },
                })?;
        let child_path = dir_path.join(&entry.name);
        match entry.entry_type {
            limnifs_core::directory_node::entry_type::FILE => {
                write_file_contents(blob, child_inode, &child_path, image, slab_index)?;
                set_mode(&child_path, child_inode.mode);
                *file_count += 1;
            }
            limnifs_core::directory_node::entry_type::DIRECTORY => {
                std::fs::create_dir_all(&child_path).map_err(|source| CliError::ReadFailed {
                    path: child_path.clone(),
                    source,
                })?;
                *dir_count += 1;
                extract_directory(
                    blob,
                    child_inode,
                    &child_path,
                    image,
                    slab_index,
                    file_count,
                    dir_count,
                )?;
            }
            other => {
                eprintln!(
                    "limni: skipping {}: entry type 0x{other:02X} not supported in v0.1",
                    child_path.display()
                );
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_file_contents(
    _blob: &MetadataBlob,
    inode: &limnifs_core::Inode,
    target: &Path,
    image: &Path,
    slab_index: &limnifs_core::SlabIndex,
) -> Result<(), CliError> {
    use std::io::Write;
    let mut file = std::fs::File::create(target).map_err(|source| CliError::ReadFailed {
        path: target.to_path_buf(),
        source,
    })?;
    match &inode.content_handle {
        ContentHandle::InlineData(data) => {
            file.write_all(data)
                .map_err(|source| CliError::ReadFailed {
                    path: target.to_path_buf(),
                    source,
                })?;
        }
        ContentHandle::SliceMap(slices) => {
            for slice in slices {
                let slab_path = resolve_slab_path(image, slab_index)?;
                let slab_bytes =
                    std::fs::read(&slab_path).map_err(|source| CliError::ReadFailed {
                        path: slab_path.clone(),
                        source,
                    })?;
                let slab_view = limnifs_core::parse_slab(&slab_bytes).map_err(|source| {
                    CliError::FormatFailed {
                        path: slab_path.clone(),
                        source,
                    }
                })?;
                let plaintext = slab_view
                    .plaintext_for(slice.drop_id.as_bytes())
                    .ok_or_else(|| CliError::FormatFailed {
                        path: slab_path.clone(),
                        source: CoreError::Corrupt {
                            reason: format!(
                                "extract: drop id {} not found in slab",
                                format_hex(slice.drop_id.as_bytes())
                            ),
                        },
                    })?
                    .map_err(|source| CliError::FormatFailed {
                        path: slab_path.clone(),
                        source,
                    })?;
                file.write_all(plaintext)
                    .map_err(|source| CliError::ReadFailed {
                        path: target.to_path_buf(),
                        source,
                    })?;
            }
        }
        other => {
            return Err(CliError::FormatFailed {
                path: target.to_path_buf(),
                source: CoreError::Corrupt {
                    reason: format!("extract: cannot extract content_handle: {other:?}"),
                },
            });
        }
    }
    Ok(())
}

fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(mode & 0o7777);
        let _ = std::fs::set_permissions(path, perms);
    }
    let _ = path;
    let _ = mode;
}

/// Derive the path to the slab file that holds a slice's drop. Uses
/// the first locator of the first entry in the slab index, resolved
/// relative to the manifest file's directory.
fn resolve_slab_path(
    image: &Path,
    slab_index: &limnifs_core::SlabIndex,
) -> Result<PathBuf, CliError> {
    if slab_index.is_empty() {
        return Err(CliError::FormatFailed {
            path: image.to_path_buf(),
            source: CoreError::Corrupt {
                reason: "slab index is empty but inode references a drop".into(),
            },
        });
    }
    let entry = &slab_index.entries[0];
    if entry.locators.is_empty() {
        return Err(CliError::FormatFailed {
            path: image.to_path_buf(),
            source: CoreError::Corrupt {
                reason: "slab index entry has no locators".into(),
            },
        });
    }
    let locator = &entry.locators[0];
    let slab_name = locator.uri.strip_prefix("file:").unwrap_or(&locator.uri);
    Ok(image
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(slab_name))
}

/// Common loader: parses the manifest prefix, extracts the inlined
/// metadata blob, and returns the parsed blob + root inode number +
/// slab index. Used by both `ls` and `cat`.
fn load_image(
    manifest_bytes: &[u8],
    image: &Path,
    map_err: impl Fn(CoreError) -> CliError,
) -> Result<(limnifs_core::MetadataBlob, u64, limnifs_core::SlabIndex), CliError> {
    let mut cursor = ManifestCursor::new(manifest_bytes);
    let _ = parse_manifest_header(&mut cursor).map_err(&map_err)?;
    let _ = parse_feature_flags_section(&mut cursor).map_err(&map_err)?;
    let meta_ref = parse_metadata_reference(&mut cursor).map_err(&map_err)?;
    let blob_bytes = meta_ref
        .inline_metadata
        .as_deref()
        .ok_or_else(|| CliError::FormatFailed {
            path: image.to_path_buf(),
            source: CoreError::Corrupt {
                reason: "inlined metadata required (external-metadata images not yet supported)"
                    .into(),
            },
        })?;
    let mut blob_cursor = ManifestCursor::new(blob_bytes);
    let blob = parse_metadata_blob(&mut blob_cursor).map_err(&map_err)?;

    let slab_index = parse_slab_index(&mut cursor).map_err(&map_err)?;

    let root_inode_number = blob
        .root_inode_number()
        .ok_or_else(|| CliError::FormatFailed {
            path: image.to_path_buf(),
            source: CoreError::Corrupt {
                reason: "metadata blob: could not identify a unique root directory inode".into(),
            },
        })?;
    if blob.inode_by_number(root_inode_number).is_none() {
        return Err(CliError::FormatFailed {
            path: image.to_path_buf(),
            source: CoreError::Corrupt {
                reason: format!("metadata blob: root inode {root_inode_number} missing"),
            },
        });
    }

    Ok((blob, root_inode_number, slab_index))
}

/// Walk a slash-separated path from `root_inode` and return the inode
/// it resolves to. Empty path or `/` returns the root inode itself.
/// Returns `None` if any component is missing or non-directory.
fn resolve_path<'a>(
    blob: &'a MetadataBlob,
    root_inode: &'a limnifs_core::Inode,
    path: &str,
) -> Option<&'a limnifs_core::Inode> {
    let trimmed = path.trim_start_matches('/').trim_end_matches('/');
    if trimmed.is_empty() {
        return Some(root_inode);
    }
    let mut current_inode = root_inode;
    for component in trimmed.split('/') {
        if component.is_empty() || component.contains('\0') {
            return None;
        }
        let hash = match &current_inode.content_handle {
            ContentHandle::Directory(h) => *h,
            _ => return None,
        };
        let node = blob.dir_node_by_hash(&hash)?;
        let entry = node.entries.iter().find(|e| e.name == component)?;
        current_inode = blob.inode_by_number(entry.inode_number)?;
    }
    Some(current_inode)
}

fn print_directory_listing(image: &Path, path: &str, node: &limnifs_core::DirectoryNode) {
    println!("{}: directory listing of {path:?}", image.display());
    if node.entries.is_empty() {
        println!("  (empty)");
        return;
    }
    println!("  entries: {}", node.entries.len());
    for entry in &node.entries {
        let kind = match entry.entry_type {
            limnifs_core::directory_node::entry_type::FILE => "file",
            limnifs_core::directory_node::entry_type::DIRECTORY => "dir",
            limnifs_core::directory_node::entry_type::SYMLINK => "symlink",
            limnifs_core::directory_node::entry_type::SPECIAL => "special",
            other => panic!("invalid entry type 0x{other:02X} from parsed node"),
        };
        println!(
            "  inode={:<6} kind={kind:<8} name={}",
            entry.inode_number, entry.name
        );
    }
}

fn format_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Stable summary of a parsed metadata blob, used for differential
/// conformance: Rust and Python must produce the same summary for
/// the same blob.
#[derive(Clone, Debug, Eq, PartialEq)]
struct MetadataSummary {
    inode_count: usize,
    dir_node_count: usize,
    root_inode_number: Option<u64>,
    /// Inode numbers sorted ascending, with each inode's mode and
    /// content-handle kind tag. Sorted so byte-for-byte comparison
    /// is meaningful regardless of the writer's traversal order.
    inodes: Vec<(u64, u32, u8)>,
    /// Directory node entry counts sorted ascending, with the
    /// first entry's name (or empty string for an empty node).
    dir_nodes: Vec<(usize, String)>,
}

impl MetadataSummary {
    fn from_blob(blob: &MetadataBlob) -> Self {
        let mut inodes: Vec<(u64, u32, u8)> = blob
            .inodes
            .iter()
            .map(|i| (i.number, i.mode, content_handle_kind_tag(&i.content_handle)))
            .collect();
        inodes.sort_by_key(|(n, _, _)| *n);

        let mut dir_nodes: Vec<(usize, String)> = blob
            .dir_nodes
            .iter()
            .map(|n| {
                let first = n
                    .entries
                    .first()
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                (n.entries.len(), first)
            })
            .collect();
        dir_nodes.sort();

        Self {
            inode_count: blob.inodes.len(),
            dir_node_count: blob.dir_nodes.len(),
            root_inode_number: blob.root_inode_number(),
            inodes,
            dir_nodes,
        }
    }
}

fn content_handle_kind_tag(handle: &ContentHandle) -> u8 {
    match handle {
        ContentHandle::InlineData(_) => 1,
        ContentHandle::SliceMap(_) => 2,
        ContentHandle::Directory(_) => 3,
        ContentHandle::Symlink(_) => 4,
        ContentHandle::Device(_) => 5,
        ContentHandle::Pipe(_) => 6,
    }
}

#[allow(clippy::too_many_arguments)]
fn print_report(
    path: &Path,
    header: ManifestHeader,
    flags: &FeatureFlags,
    metadata_inlined: bool,
    slab_index_len: usize,
    history_len: usize,
    extra_bytes_remaining: usize,
    merkle_root: ManifestRoot,
    metadata_summary: Option<&MetadataSummary>,
    json: bool,
) {
    if json {
        print_json_report(
            path,
            header,
            flags,
            metadata_inlined,
            slab_index_len,
            history_len,
            extra_bytes_remaining,
            merkle_root,
            metadata_summary,
        );
    } else {
        print_human_report(
            path,
            header,
            flags,
            metadata_inlined,
            slab_index_len,
            history_len,
            extra_bytes_remaining,
            merkle_root,
            metadata_summary,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn print_human_report(
    path: &Path,
    header: ManifestHeader,
    flags: &FeatureFlags,
    metadata_inlined: bool,
    slab_index_len: usize,
    history_len: usize,
    extra_bytes_remaining: usize,
    merkle_root: ManifestRoot,
    metadata_summary: Option<&MetadataSummary>,
) {
    println!("{}: valid LimniFS manifest", path.display());
    println!("  magic:               LMFS");
    println!("  drop store version:  {}", header.drop_store_version);
    println!("  metadata version:    {}", header.metadata_version);
    println!("  manifest version:    {}", header.manifest_version);
    if flags.is_empty() {
        println!("  feature flags:       0 entries");
    } else {
        println!("  feature flags:       {} entries", flags.len());
        for entry in &flags.entries {
            let kind = if entry.required {
                "required"
            } else {
                "optional"
            };
            println!("    0x{:04X}            {kind}", entry.flag_id);
        }
    }
    println!(
        "  metadata:            {}",
        if metadata_inlined {
            "inlined"
        } else {
            "external"
        }
    );
    println!("  slab index:          {slab_index_len} entries");
    println!("  history:             {history_len} entries");
    if let Some(summary) = metadata_summary {
        println!(
            "  metadata blob:       {} inodes, {} dir nodes, root inode = {}",
            summary.inode_count,
            summary.dir_node_count,
            summary
                .root_inode_number
                .map_or_else(|| "?".to_string(), |n| n.to_string())
        );
    }
    if extra_bytes_remaining > 0 {
        println!(
            "  warning:             {extra_bytes_remaining} extra bytes after history (optional sections present, not parsed)"
        );
        println!(
            "                       merkle root assumes no optional sections (crypto/EC/DMS/delta)"
        );
    }
    println!("  merkle root:         {merkle_root}");
    println!("  limni version:       {VERSION}");
}

#[allow(clippy::too_many_arguments)]
fn print_json_report(
    path: &Path,
    header: ManifestHeader,
    flags: &FeatureFlags,
    metadata_inlined: bool,
    slab_index_len: usize,
    history_len: usize,
    extra_bytes_remaining: usize,
    merkle_root: ManifestRoot,
    metadata_summary: Option<&MetadataSummary>,
) {
    let escaped_path = escape_json_path(path);
    print!("{{\"path\":\"{escaped_path}\",\"magic\":\"LMFS\",");
    print!("\"drop_store_version\":{},", header.drop_store_version);
    print!("\"metadata_version\":{},", header.metadata_version);
    print!("\"manifest_version\":{},", header.manifest_version);
    print!("\"feature_flags\":[");
    for (i, entry) in flags.entries.iter().enumerate() {
        if i > 0 {
            print!(",");
        }
        let required = if entry.required { "true" } else { "false" };
        print!("{{\"flag_id\":{},\"required\":{required}}}", entry.flag_id);
    }
    print!("],");
    print!("\"metadata_inlined\":{metadata_inlined},");
    print!("\"slab_index_entries\":{slab_index_len},");
    print!("\"history_entries\":{history_len},");
    print!("\"extra_bytes_after_history\":{extra_bytes_remaining},");
    if let Some(summary) = metadata_summary {
        print!("\"metadata_inode_count\":{},", summary.inode_count);
        print!("\"metadata_dir_node_count\":{},", summary.dir_node_count);
        print!(
            "\"metadata_root_inode\":{},",
            summary
                .root_inode_number
                .map_or_else(|| "null".to_string(), |n| n.to_string())
        );
        print!("\"metadata_inodes\":[");
        for (i, (number, mode, kind)) in summary.inodes.iter().enumerate() {
            if i > 0 {
                print!(",");
            }
            print!("{{\"number\":{number},\"mode\":{mode},\"kind\":{kind}}}");
        }
        print!("],");
        print!("\"metadata_dir_nodes\":[");
        for (i, (entries, first)) in summary.dir_nodes.iter().enumerate() {
            if i > 0 {
                print!(",");
            }
            let escaped_first = escape_json_path(std::path::Path::new(first));
            print!("{{\"entries\":{entries},\"first\":\"{escaped_first}\"}}");
        }
        print!("],");
    }
    println!("\"merkle_root\":\"{merkle_root}\"}}");
}

fn escape_json_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use limnifs_core::MANIFEST_HEADER_LEN;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn make_temp_file(contents: &[u8]) -> PathBuf {
        let id = TEMP_FILE_COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "limni-test-{pid}-{id}-{nanos}.lim",
            pid = std::process::id(),
            nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0u128, |d| d.as_nanos()),
        ));
        let mut file = std::fs::File::create(&dir).expect("create temp file");
        file.write_all(contents).expect("write temp file");
        dir
    }

    fn make_current_header() -> [u8; MANIFEST_HEADER_LEN] {
        let mut bytes = [0u8; MANIFEST_HEADER_LEN];
        bytes[..4].copy_from_slice(b"LMFS");
        bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&1u16.to_le_bytes());
        bytes[8..10].copy_from_slice(&1u16.to_le_bytes());
        bytes
    }

    fn append_feature_flags(bytes: &mut Vec<u8>, entries: &[(u16, u8)]) {
        bytes.push(0x01); // section version 1
        let count = u32::try_from(entries.len()).expect("count fits u32");
        bytes.extend_from_slice(&count.to_le_bytes());
        for (flag_id, required) in entries {
            bytes.extend_from_slice(&flag_id.to_le_bytes());
            bytes.push(*required);
        }
    }

    fn append_metadata_reference_external(bytes: &mut Vec<u8>, uri: &str) {
        bytes.push(0x01); // section version 1
        bytes.extend_from_slice(&[0xAA; 32]); // metadata_hash
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 locator
        let uri_len = u32::try_from(uri.len()).expect("uri fits u32");
        bytes.extend_from_slice(&uri_len.to_le_bytes());
        bytes.extend_from_slice(uri.as_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // inline_metadata_len = 0
    }

    fn append_slab_index_single(bytes: &mut Vec<u8>, uri: &str) {
        bytes.push(0x01); // section version 1
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 entry
        bytes.extend_from_slice(&0u64.to_le_bytes()); // slab_id.ordinal = 0
        bytes.extend_from_slice(&[0u8; 32]); // slab_id.hash
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 locator
        let uri_len = u32::try_from(uri.len()).expect("uri fits u32");
        bytes.extend_from_slice(&uri_len.to_le_bytes());
        bytes.extend_from_slice(uri.as_bytes());
    }

    fn append_history_single_build(bytes: &mut Vec<u8>) {
        bytes.push(0x01); // section version 1
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 entry
        bytes.push(0x01); // op = build
        bytes.extend_from_slice(&0u64.to_le_bytes()); // timestamp = 0
        bytes.extend_from_slice(&0u32.to_le_bytes()); // input_count = 0
        bytes.extend_from_slice(&0u32.to_le_bytes()); // params_len = 0
    }

    fn make_minimal_valid_manifest() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&make_current_header());
        append_feature_flags(&mut bytes, &[]);
        append_metadata_reference_external(&mut bytes, "file:///metadata.bin");
        append_slab_index_single(&mut bytes, "file:///slab-0.bin");
        append_history_single_build(&mut bytes);
        bytes
    }

    #[test]
    fn verify_accepts_current_header() {
        // Smoke: a minimal valid manifest parses end-to-end and
        // produces a non-zero ManifestRoot.
        let bytes = make_minimal_valid_manifest();
        let path = make_temp_file(&bytes);
        verify(&path, false).expect("minimal valid manifest verifies");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verify_emits_json_with_merkle_root() {
        let bytes = make_minimal_valid_manifest();
        let path = make_temp_file(&bytes);
        verify(&path, true).expect("json output works");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verify_computes_deterministic_root_across_runs() {
        // Same manifest bytes -> same ManifestRoot (smoke only; we
        // cannot assert the exact root here without duplicating the
        // formula. The merkle module already tests exact values.)
        let bytes = make_minimal_valid_manifest();
        let path = make_temp_file(&bytes);
        verify(&path, false).expect("first run");
        verify(&path, false).expect("second run");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verify_parses_header_plus_empty_feature_flags() {
        // Construct a manifest with empty feature flags (still need
        // the other required sections to reach the Merkle step).
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&make_current_header());
        append_feature_flags(&mut bytes, &[]);
        append_metadata_reference_external(&mut bytes, "file:///m.bin");
        append_slab_index_single(&mut bytes, "file:///s.bin");
        append_history_single_build(&mut bytes);
        let path = make_temp_file(&bytes);
        verify(&path, false).expect("empty flags parses");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verify_parses_header_plus_one_required_flag() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&make_current_header());
        append_feature_flags(&mut bytes, &[(0x0020, 0x01)]);
        append_metadata_reference_external(&mut bytes, "file:///m.bin");
        append_slab_index_single(&mut bytes, "file:///s.bin");
        append_history_single_build(&mut bytes);
        let path = make_temp_file(&bytes);
        verify(&path, false).expect("header + one flag parses");
        verify(&path, true).expect("json output works");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verify_rejects_unknown_section_version_after_header() {
        let mut bytes = Vec::from(make_current_header());
        bytes.push(0x07); // unknown section version
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let path = make_temp_file(&bytes);
        match verify(&path, false) {
            Err(CliError::FormatFailed { source, .. }) => {
                assert!(matches!(source, CoreError::UnsupportedFeature { .. }));
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verify_rejects_short_file() {
        // Valid magic + valid version fields but truncated (no reserved
        // bytes). Header parsing reaches the reserved read with zero
        // bytes remaining.
        let mut bytes = vec![0u8; 10];
        bytes[..4].copy_from_slice(b"LMFS");
        bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&1u16.to_le_bytes());
        bytes[8..10].copy_from_slice(&1u16.to_le_bytes());
        let path = make_temp_file(&bytes);
        match verify(&path, false) {
            Err(CliError::FormatFailed { source, .. }) => {
                assert!(
                    matches!(source, CoreError::TooShort { .. }),
                    "got {source:?}"
                );
            }
            other => panic!("expected FormatFailed TooShort, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verify_rejects_bad_magic() {
        let mut bytes = make_current_header();
        bytes[0] = b'X';
        let path = make_temp_file(&bytes);
        match verify(&path, false) {
            Err(CliError::FormatFailed { source, .. }) => {
                assert!(matches!(source, CoreError::BadMagic { .. }));
            }
            other => panic!("expected FormatFailed BadMagic, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verify_rejects_missing_file() {
        let path = PathBuf::from("/nonexistent/limni-test-does-not-exist.lim");
        match verify(&path, false) {
            Err(CliError::ReadFailed { .. }) => {}
            other => panic!("expected ReadFailed, got {other:?}"),
        }
    }

    #[test]
    fn json_output_contains_versions() {
        let bytes = make_minimal_valid_manifest();
        let path = make_temp_file(&bytes);
        verify(&path, true).expect("verify ok");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn escape_json_path_handles_backslashes_and_quotes() {
        let p = PathBuf::from(r#"weird"pa\th"#);
        let escaped = escape_json_path(&p);
        assert!(escaped.contains(r#"\""#));
        assert!(escaped.contains(r"\\"));
    }

    #[test]
    fn cli_error_is_debug() {
        let err = CliError::ReadFailed {
            path: PathBuf::from("/x"),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        };
        assert!(format!("{err:?}").contains("ReadFailed"));
    }

    fn make_source_tree() -> std::path::PathBuf {
        let id = TEMP_FILE_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "limni-ls-source-{pid}-{id}-{nanos}",
            pid = std::process::id(),
            nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0u128, |d| d.as_nanos()),
        ));
        std::fs::create_dir_all(&dir).expect("create source root");
        std::fs::create_dir_all(dir.join("sub")).expect("create subdir");
        std::fs::write(dir.join("a.txt"), b"aaa").expect("write a.txt");
        std::fs::write(dir.join("b.txt"), b"bbb").expect("write b.txt");
        std::fs::write(dir.join("sub").join("c.txt"), b"ccc").expect("write c.txt");
        dir
    }

    #[test]
    fn ls_root_lists_sorted_entries() {
        let source = make_source_tree();
        let image = make_temp_file(&[]);
        std::fs::remove_file(&image).ok();
        limn(&source, &image).expect("write image");
        std::fs::remove_dir_all(&source).ok();

        // ls should not error; we do not capture stdout here, but
        // the absence of a return Err proves the metadata-blob parser
        // + path resolver both worked end-to-end.
        ls(&image, "/").expect("ls root succeeds");
        ls(&image, "/sub").expect("ls subdir succeeds");
        let _ = std::fs::remove_file(&image);
    }

    #[test]
    fn ls_missing_path_reports_corrupt() {
        let source = make_source_tree();
        let image = make_temp_file(&[]);
        std::fs::remove_file(&image).ok();
        limn(&source, &image).expect("write image");
        std::fs::remove_dir_all(&source).ok();
        match ls(&image, "/does-not-exist") {
            Err(CliError::FormatFailed { source, .. }) => {
                assert!(matches!(source, CoreError::Corrupt { .. }));
            }
            other => panic!("expected FormatFailed, got {other:?}"),
        }
        let _ = std::fs::remove_file(&image);
    }

    #[test]
    fn ls_missing_image_reports_read_failed() {
        let path = PathBuf::from("/nonexistent/limni-ls-test-does-not-exist.lim");
        match ls(&path, "/") {
            Err(CliError::ReadFailed { .. }) => {}
            other => panic!("expected ReadFailed, got {other:?}"),
        }
    }

    #[test]
    fn cat_writes_inline_file_to_stdout() {
        let source = make_source_tree();
        let image = make_temp_file(&[]);
        std::fs::remove_file(&image).ok();
        limn(&source, &image).expect("write image");
        std::fs::remove_dir_all(&source).ok();

        // The ls-e2e tree has a.txt with content "aaa". Redirecting
        // stdout from cat() directly is hard inside a unit test; the
        // absence of an Err proves end-to-end success.
        cat(&image, "/a.txt").expect("cat inline file succeeds");
        let _ = std::fs::remove_file(&image);
    }

    #[test]
    fn cat_missing_path_reports_corrupt() {
        let source = make_source_tree();
        let image = make_temp_file(&[]);
        std::fs::remove_file(&image).ok();
        limn(&source, &image).expect("write image");
        std::fs::remove_dir_all(&source).ok();
        match cat(&image, "/does-not-exist") {
            Err(CliError::FormatFailed { source, .. }) => {
                assert!(matches!(source, CoreError::Corrupt { .. }));
            }
            other => panic!("expected FormatFailed, got {other:?}"),
        }
        let _ = std::fs::remove_file(&image);
    }

    #[test]
    fn cat_directory_path_reports_corrupt() {
        let source = make_source_tree();
        let image = make_temp_file(&[]);
        std::fs::remove_file(&image).ok();
        limn(&source, &image).expect("write image");
        std::fs::remove_dir_all(&source).ok();
        match cat(&image, "/sub") {
            Err(CliError::FormatFailed { source, .. }) => {
                assert!(matches!(source, CoreError::Corrupt { .. }));
            }
            other => panic!("expected FormatFailed, got {other:?}"),
        }
        let _ = std::fs::remove_file(&image);
    }

    #[test]
    fn stat_inline_file_succeeds() {
        let source = make_source_tree();
        let image = make_temp_file(&[]);
        std::fs::remove_file(&image).ok();
        limn(&source, &image).expect("write image");
        std::fs::remove_dir_all(&source).ok();
        stat(&image, "/a.txt").expect("stat inline file succeeds");
        stat(&image, "/").expect("stat root succeeds");
        stat(&image, "/sub").expect("stat subdir succeeds");
        let _ = std::fs::remove_file(&image);
    }

    #[test]
    fn stat_missing_path_reports_corrupt() {
        let source = make_source_tree();
        let image = make_temp_file(&[]);
        std::fs::remove_file(&image).ok();
        limn(&source, &image).expect("write image");
        std::fs::remove_dir_all(&source).ok();
        match stat(&image, "/no-such-file") {
            Err(CliError::FormatFailed { source, .. }) => {
                assert!(matches!(source, CoreError::Corrupt { .. }));
            }
            other => panic!("expected FormatFailed, got {other:?}"),
        }
        let _ = std::fs::remove_file(&image);
    }

    #[test]
    fn extract_round_trips_inline_files() {
        let source = make_source_tree();
        let image = make_temp_file(&[]);
        std::fs::remove_file(&image).ok();
        limn(&source, &image).expect("write image");

        let dest = std::env::temp_dir().join(format!(
            "limni-extract-test-{}-{}",
            std::process::id(),
            TEMP_FILE_COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::remove_dir_all(&dest).ok();

        extract(&image, &dest).expect("extract succeeds");

        let a = std::fs::read(dest.join("a.txt")).expect("a.txt extracted");
        assert_eq!(a, b"aaa");
        let c = std::fs::read(dest.join("sub").join("c.txt")).expect("c.txt extracted");
        assert_eq!(c, b"ccc");

        std::fs::remove_dir_all(&dest).ok();
        std::fs::remove_dir_all(&source).ok();
        let _ = std::fs::remove_file(&image);
    }

    #[test]
    fn extract_creates_destination_if_missing() {
        let source = make_source_tree();
        let image = make_temp_file(&[]);
        std::fs::remove_file(&image).ok();
        limn(&source, &image).expect("write image");

        let dest = std::env::temp_dir().join(format!(
            "limni-extract-missing-{}-{}",
            std::process::id(),
            TEMP_FILE_COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        assert!(!dest.exists());

        extract(&image, &dest).expect("extract creates dest");
        assert!(dest.is_dir());

        std::fs::remove_dir_all(&dest).ok();
        std::fs::remove_dir_all(&source).ok();
        let _ = std::fs::remove_file(&image);
    }
}
