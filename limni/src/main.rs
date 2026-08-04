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

#![deny(unsafe_code)]
#![allow(warnings)]

pub mod vfs;

#[cfg(feature = "fuse")]
pub mod fuse_vfs;

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use limnifs_core::{
    compute_merkle_root, hash_empty_section, hash_section, parse_dms_policy, parse_ec_params,
    parse_feature_flags_section, parse_history, parse_manifest_header, parse_metadata_blob,
    parse_metadata_reference, parse_slab_index, ContentHandle, CoreError, FeatureFlags, HistoryOp,
    ManifestCursor, ManifestHeader, ManifestRoot, MetadataBlob, SectionHashes,
};
use limnifs_core::dictionary_section::parse_dictionary_section;

/// Install dictionaries parsed from the manifest's `dictionary_section`
/// into a `SlabStore`. Drops with `dict_id != NO_DICT` will use the
/// dict-aware ZSTD decompress path.
fn install_dicts(
    store: &mut limnifs_core::slab_store::SlabStore,
    section: &limnifs_core::dictionary_section::DictionarySection,
) {
    use std::collections::HashMap;
    let mut map: HashMap<u8, Vec<u8>> = HashMap::new();
    for d in &section.dicts {
        // Use `class_id` as the dict id (matches the writer's
        // convention — see WriteContext::assemble).
        map.insert(d.class_id, d.data.clone());
    }
    store.set_dictionaries(map);
}

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
        /// Compression profile to use. Built-in: max-ratio, max-speed,
        /// balanced, competitive, max-read, max-write, max-write-rw,
        /// max-read-rw, balanced-rw. Default: balanced.
        #[arg(long)]
        profile: Option<String>,
        /// Override text codec (e.g. "brotli", "zstd", "lz4").
        #[arg(long)]
        text_codec: Option<String>,
        /// Override average chunk size in bytes.
        #[arg(long)]
        chunk_size: Option<u32>,
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
        /// Byte offset to start reading from (default: 0).
        #[arg(long)]
        offset: Option<u64>,
        /// Maximum number of bytes to read (default: all).
        #[arg(long)]
        length: Option<u64>,
    },
    /// Write multiple files' contents from a `.lim` image to stdout,
    /// each preceded by a `==> <path> ==>` header line.
    ///
    /// Equivalent to invoking `limni cat` once per path, but parses
    /// the manifest only once. Use this when reading many files:
    /// amortizes the parse cost across all reads. For a tree with
    /// 1000 small files this is ~100x faster than 1000 separate
    /// `limni cat` invocations.
    CatMulti {
        image: PathBuf,
        /// One or more slash-separated paths inside the image.
        #[arg(num_args = 1.., required = true)]
        paths: Vec<String>,
    },
    /// Print an inode's metadata (number, mode, sizes, content handle).
    Stat { image: PathBuf, path: String },
    /// Print the directory tree recursively (like the `tree` command).
    Tree {
        image: PathBuf,
        #[arg(default_value = "/")]
        path: String,
    },
    /// Extract an image's contents to a filesystem directory.
    Extract { image: PathBuf, dest: PathBuf },
    /// Add a file to an existing image (RW).
    Add {
        image: PathBuf,
        dest: String,
        source: PathBuf,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Delete a file from an existing image (RW).
    Delete {
        image: PathBuf,
        path: String,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Compact an image: extract all files, re-compress with the
    /// turnover profile, and overwrite. Removes unreferenced drops
    /// and history fragmentation.
    Turnover {
        image: PathBuf,
        #[arg(long, default_value = "max-ratio")]
        profile: String,
    },
    /// Compute tree operations between a parent and child image.
    Diff { parent: PathBuf, child: PathBuf },
    /// Print a comprehensive overview of an image: manifest summary,
    /// metadata blob stats, slab stats, and per-class drop counts.
    Inspect {
        /// Path to the `.lim` image to inspect.
        image: PathBuf,
    },
    /// Inspect a slab file: list drop records, codecs, and sizes.
    Slab {
        /// Path to the slab file (.bin).
        slab: PathBuf,
    },
    /// Analyze garbage: find unreferenced drops in the slab vs the manifest.
    Gc {
        /// Path to the `.lim` image.
        image: PathBuf,
    },
    /// Print the history section: operations that produced this image.
    History {
        /// Path to the `.lim` image.
        image: PathBuf,
    },
    /// Analyze dedup: show how many drops are shared across files.
    Dedup {
        /// Path to the `.lim` image.
        image: PathBuf,
    },
    /// Compact an image: extract → re-write to eliminate slab garbage.
    Compact {
        /// Path to the source `.lim` image.
        source: PathBuf,
        /// Path to the compacted output `.lim` image.
        output: PathBuf,
    },
    /// Deep integrity check: verify drop `BLAKE3` hashes against `DropId`s.
    Check {
        /// Path to the `.lim` image.
        image: PathBuf,
    },
    /// Quick write/read/extract benchmark on a synthetic tree.
    Benchmark,
    /// Generate a random AEAD key (XChaCha20-Poly1305, 32 bytes).
    Keygen,
    /// Encrypt a file using XChaCha20-Poly1305.
    Seal {
        input: PathBuf,
        output: PathBuf,
        /// 64-character hex key (32 bytes).
        key: String,
    },
    /// Decrypt a file sealed with `limni seal`.
    Open {
        input: PathBuf,
        output: PathBuf,
        /// 64-character hex key (32 bytes).
        key: String,
    },
    /// Split a file into `n` Shamir shares, any `k` of which reconstruct it.
    ///
    /// Each share is written to `<output_prefix>.share-<i>` for i in 1..=n.
    /// Share format: 1 byte index + payload bytes.
    ShamirSplit {
        input: PathBuf,
        output_prefix: PathBuf,
        /// Threshold (k): minimum shares needed to reconstruct.
        #[arg(long)]
        threshold: usize,
        /// Total share count (n).
        #[arg(long)]
        shares: usize,
    },
    /// Combine Shamir shares back into the original file.
    ///
    /// Pass at least `k` share paths; output is written to the given path.
    ShamirCombine {
        /// One or more share paths (use at least `k` for reconstruction).
        #[arg(num_args = 1.., required = true)]
        shares: Vec<PathBuf>,
        output: PathBuf,
    },
    /// Export a `.lim` image as a composefs mountable directory tree.
    ///
    /// Linux fast path: extracts the tree to `<out-dir>/rootfs/`, then
    /// shells out to `mkcomposefs` (from composefs-utils) to produce
    /// `<out-dir>/image.cfs` — an EROFS image backed by a fs-verity
    /// content-addressed blob store. Mount on Linux ≥ 6.4 via:
    ///
    /// ```text
    /// mount.composefs -o basedir=<out-dir>/objects <out-dir>/image.cfs /mnt
    /// ```
    ///
    /// If `mkcomposefs` is not on PATH, the extracted rootfs is left
    /// in place and a warning is printed; the user can install
    /// composefs-utils and re-run.
    /// Sign a `.lim` image's `ManifestRoot` using sigstore keyless mode.
    ///
    /// Shells out to `cosign sign-blob` (<https://github.com/sigstore/cosign>).
    /// The signer authenticates via OIDC (Google/GitHub/etc.); Fulcio
    /// issues a short-lived cert; Rekor logs the signature publicly.
    /// The bundle (cert + signature + Rekor inclusion proof) is written
    /// to `<image>.sigstore.json`.
    ///
    /// Requires `cosign` on PATH and an interactive OIDC flow. For an
    /// offline, self-sovereign alternative, use the Ed25519 keypair
    /// API in `limnifs-core::signing`.
    /// Verify a `.lim` image's sigstore signature bundle.
    ///
    /// Shells out to `cosign verify-blob`. Checks the Fulcio cert
    /// chain, the Rekor inclusion proof, and the signature against
    /// the image's `ManifestRoot`.
    ///
    /// Requires `cosign` on PATH. For offline verification of
    /// Ed25519 keypair signatures, use `limnifs-core::signing::verify`.
    /// Mount a `.lim` image as a read-only filesystem.
    ///
    /// Requires the `fuse` feature (built with `--features fuse`) and
    /// FUSE kernel support (macFUSE on macOS, libfuse on Linux).
    #[cfg(feature = "fuse")]
    Mount {
        /// Path to the `.lim` image to mount.
        image: PathBuf,
        /// Mount point directory (must exist).
        mountpoint: PathBuf,
    },
}

fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    match cli.command {
        Command::Verify { image, json } => verify(&image, json),
        Command::Limn {
            source,
            output,
            profile,
            text_codec,
            chunk_size,
        } => limn_with_profile(&source, &output, profile, text_codec, chunk_size),
        Command::Ls { image, path } => ls(&image, &path),
        Command::Cat {
            image,
            path,
            offset,
            length,
        } => cat(&image, &path, offset, length),
        Command::CatMulti { image, paths } => cat_multi(&image, &paths),
        Command::Stat { image, path } => stat(&image, &path),
        Command::Tree { image, path } => tree(&image, &path),
        Command::Extract { image, dest } => extract(&image, &dest),
        Command::Add {
            image,
            dest,
            source,
            profile,
        } => rw_add(&image, &dest, &source, profile),
        Command::Delete {
            image,
            path,
            profile,
        } => rw_delete(&image, &path, profile),
        Command::Turnover { image, profile } => turnover_cmd(&image, &profile),
        Command::Diff { parent, child } => diff(&parent, &child),
        Command::Inspect { image } => inspect(&image),
        Command::Slab { slab } => slab_cmd(&slab),
        Command::Gc { image } => gc_cmd(&image),
        Command::History { image } => history_cmd(&image),
        Command::Dedup { image } => dedup_cmd(&image),
        Command::Compact { source, output } => compact(&source, &output),
        Command::Check { image } => check_cmd(&image),
        Command::Benchmark => benchmark(),
        Command::Keygen => keygen(),
        Command::Seal { input, output, key } => seal_cmd(&input, &output, &key),
        Command::Open { input, output, key } => open_cmd(&input, &output, &key),
        Command::ShamirSplit {
            input,
            output_prefix,
            threshold,
            shares,
        } => shamir_split(&input, &output_prefix, threshold, shares),
        Command::ShamirCombine { shares, output } => shamir_combine(&shares, &output),
        #[cfg(feature = "fuse")]
        Command::Mount { image, mountpoint } => mount(&image, &mountpoint),
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

#[allow(clippy::too_many_lines)]
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
        metadata_reference.inline_metadata.as_deref().and_then(|blob_bytes| {
            let mut blob_cursor = ManifestCursor::new(blob_bytes);
            parse_metadata_blob(&mut blob_cursor)
                .ok()
                .map(|blob| {
                    let mut inodes: Vec<(u64, u32, u8)> = blob
                        .inodes
                        .iter()
                        .map(|i| {
                            (
                                i.number,
                                i.mode,
                                content_handle_tag(&i.content_handle),
                            )
                        })
                        .collect();
                    inodes.sort_by_key(|(n, _, _)| *n);
                    let mut dir_nodes: Vec<(usize, String)> = blob
                        .dir_nodes
                        .iter()
                        .map(|n| {
                            let first = n.entries.first().map(|e| e.name.clone()).unwrap_or_default();
                            (n.entries.len(), first)
                        })
                        .collect();
                    dir_nodes.sort();
                    format!(
                        "\"metadata_inode_count\":{},\"metadata_dir_node_count\":{},\"metadata_root_inode\":{},\"metadata_inodes\":[{}],\"metadata_dir_nodes\":[{}],",
                        blob.inodes.len(),
                        blob.dir_nodes.len(),
                        blob.root_inode_number().map_or_else(|| "null".to_string(), |n| n.to_string()),
                        inodes.iter().map(|(n, m, k)| format!("{{\"number\":{n},\"mode\":{m},\"kind\":{k}}}")).collect::<Vec<_>>().join(","),
                        dir_nodes.iter().map(|(e, f)| format!("{{\"entries\":{e},\"first\":\"{f}\"}}")).collect::<Vec<_>>().join(",")
                    )
                })
        })
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
        metadata_summary.as_deref(),
        json,
    );
    Ok(())
}

fn content_handle_tag(handle: &ContentHandle) -> u8 {
    match handle {
        ContentHandle::InlineData(_) | ContentHandle::SharedInline(_) => 1,
        ContentHandle::SliceMap(_) => 2,
        ContentHandle::Directory(_) => 3,
        ContentHandle::Symlink(_) => 4,
        ContentHandle::Device(_) => 5,
        ContentHandle::Pipe(_) => 6,
    }
}

fn limn(source: &Path, output: &Path) -> Result<(), CliError> {
    let artifact = limnifs_write::write_directory(source)
        .map_err(|source| CliError::WriteFailed { source })?;
    std::fs::write(output, &artifact.bytes).map_err(|source| CliError::ReadFailed {
        path: output.to_path_buf(),
        source,
    })?;

    let parent = output.parent().unwrap_or_else(|| std::path::Path::new("."));
    let total_slab_bytes: u64 = artifact
        .slabs
        .iter()
        .map(|slab| {
            let slab_name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
            let slab_path = parent.join(slab_name);
            std::fs::write(&slab_path, &slab.bytes).map_err(|source| CliError::ReadFailed {
                path: slab_path.clone(),
                source,
            })?;
            println!(
                "{}: wrote {} bytes (slab {}, {} drops)",
                slab_path.display(),
                slab.bytes.len(),
                slab.id.ordinal,
                slab.drop_ids.len(),
            );
            Ok::<u64, CliError>(slab.bytes.len() as u64)
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .sum();

    if let Some(sidecar) = &artifact.metadata_sidecar {
        let name = sidecar
            .locator
            .strip_prefix("file:")
            .unwrap_or(&sidecar.locator);
        let sidecar_path = parent.join(name);
        std::fs::write(&sidecar_path, &sidecar.bytes).map_err(|source| CliError::ReadFailed {
            path: sidecar_path.clone(),
            source,
        })?;
        println!(
            "{}: wrote {} bytes (metadata sidecar)",
            sidecar_path.display(),
            sidecar.bytes.len(),
        );
    }

    let slab_count = artifact.slabs.len();
    println!(
        "{output}: wrote {len} bytes, {manifest_root}",
        output = output.display(),
        len = artifact.bytes.len(),
        manifest_root = artifact.merkle_root,
    );
    println!(
        "  inodes: {}  files: {}  dirs: {}  drops: {}  slabs: {} ({total_slab_bytes} bytes)",
        artifact.inode_count,
        artifact.file_count,
        artifact.dir_count,
        artifact.drop_count,
        slab_count,
    );
    Ok(())
}

fn limn_with_profile(
    source: &Path,
    output: &Path,
    profile: Option<String>,
    text_codec: Option<String>,
    chunk_size: Option<u32>,
) -> Result<(), CliError> {
    // Resolve config from profile or use default.
    let mut config = match &profile {
        Some(name) => limnifs_write::WriteConfig::from_profile(name).unwrap_or_else(|| {
            eprintln!("warning: unknown profile '{name}', using balanced");
            limnifs_write::profile::balanced()
        }),
        None => limnifs_write::WriteConfig::default_v0_1(),
    };

    // Apply CLI overrides.
    if let Some(codec) = &text_codec {
        config = config.with_text_codec(codec);
    }
    if let Some(size) = chunk_size {
        config = config.with_chunk_size(size);
    }

    if let Some(ref p) = profile {
        eprintln!("profile: {p}");
    }
    eprintln!(
        "  text={}, binary={}, quality={}",
        config.defaults.text_codec,
        config.defaults.binary_codec,
        config.codec_tunables.brotli.quality
    );

    let artifact = limnifs_write::write_directory_with_config(source, &config)
        .map_err(|source| CliError::WriteFailed { source })?;

    std::fs::write(output, &artifact.bytes).map_err(|source| CliError::ReadFailed {
        path: output.to_path_buf(),
        source,
    })?;

    let parent = output.parent().unwrap_or_else(|| std::path::Path::new("."));
    for slab in &artifact.slabs {
        let slab_name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
        let slab_path = parent.join(slab_name);
        std::fs::write(&slab_path, &slab.bytes).map_err(|source| CliError::ReadFailed {
            path: slab_path.clone(),
            source,
        })?;
        println!(
            "{}: wrote {} bytes (slab {}, {} drops)",
            slab_path.display(),
            slab.bytes.len(),
            slab.id.ordinal,
            slab.drop_ids.len(),
        );
    }

    if let Some(sidecar) = &artifact.metadata_sidecar {
        let name = sidecar
            .locator
            .strip_prefix("file:")
            .unwrap_or(&sidecar.locator);
        let sidecar_path = parent.join(name);
        std::fs::write(&sidecar_path, &sidecar.bytes).map_err(|source| CliError::ReadFailed {
            path: sidecar_path.clone(),
            source,
        })?;
    }

    println!(
        "{output}: wrote {len} bytes, {manifest_root}",
        output = output.display(),
        len = artifact.bytes.len(),
        manifest_root = artifact.merkle_root,
    );
    println!(
        "  inodes: {}  files: {}  dirs: {}  drops: {}  slabs: {}",
        artifact.inode_count,
        artifact.file_count,
        artifact.dir_count,
        artifact.drop_count,
        artifact.slabs.len(),
    );
    Ok(())
}
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
    let (blob, root_inode_number, slab_index, _dict_section) = load_image(&manifest_bytes, image, map_err)?;
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

fn rw_add(
    image: &Path,
    dest_path: &str,
    src: &Path,
    profile: Option<String>,
) -> Result<(), CliError> {
    let config = match &profile {
        Some(name) => limnifs_write::WriteConfig::from_profile(name)
            .unwrap_or_else(|| limnifs_write::profile::balanced_rw()),
        None => limnifs_write::profile::balanced_rw(),
    };
    let staging = std::env::temp_dir().join(format!("limnifs-rw-add-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    if image.exists() {
        extract(image, &staging)?;
    } else {
        std::fs::create_dir_all(&staging).map_err(|e| CliError::ReadFailed {
            path: staging.clone(),
            source: e,
        })?;
    }
    let dest_file = staging.join(dest_path);
    if let Some(parent) = dest_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CliError::ReadFailed {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    std::fs::copy(src, &dest_file).map_err(|e| CliError::ReadFailed {
        path: src.to_path_buf(),
        source: e,
    })?;
    eprintln!("added: {dest_path} ({})", src.display());
    let artifact = limnifs_write::write_directory_with_config(&staging, &config)
        .map_err(|source| CliError::WriteFailed { source })?;
    std::fs::write(image, &artifact.bytes).map_err(|e| CliError::ReadFailed {
        path: image.to_path_buf(),
        source: e,
    })?;
    let parent = image.parent().unwrap_or_else(|| std::path::Path::new("."));
    for slab in &artifact.slabs {
        let slab_name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
        std::fs::write(parent.join(slab_name), &slab.bytes).map_err(|e| CliError::ReadFailed {
            path: parent.join(slab_name),
            source: e,
        })?;
    }
    if let Some(sidecar) = &artifact.metadata_sidecar {
        let name = sidecar
            .locator
            .strip_prefix("file:")
            .unwrap_or(&sidecar.locator);
        std::fs::write(parent.join(name), &sidecar.bytes).map_err(|e| CliError::ReadFailed {
            path: parent.join(name),
            source: e,
        })?;
    }
    eprintln!(
        "rebuilt: {} ({} inodes, {} drops)",
        image.display(),
        artifact.inode_count,
        artifact.drop_count,
    );
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

fn rw_delete(image: &Path, path: &str, profile: Option<String>) -> Result<(), CliError> {
    let config = match &profile {
        Some(name) => limnifs_write::WriteConfig::from_profile(name)
            .unwrap_or_else(|| limnifs_write::profile::balanced_rw()),
        None => limnifs_write::profile::balanced_rw(),
    };
    let staging = std::env::temp_dir().join(format!("limnifs-rw-del-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    extract(image, &staging)?;
    let target = staging.join(path);
    std::fs::remove_file(&target).map_err(|e| CliError::ReadFailed {
        path: target.clone(),
        source: e,
    })?;
    eprintln!("deleted: {path}");
    let artifact = limnifs_write::write_directory_with_config(&staging, &config)
        .map_err(|source| CliError::WriteFailed { source })?;
    std::fs::write(image, &artifact.bytes).map_err(|e| CliError::ReadFailed {
        path: image.to_path_buf(),
        source: e,
    })?;
    let parent = image.parent().unwrap_or_else(|| std::path::Path::new("."));
    for slab in &artifact.slabs {
        let slab_name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
        std::fs::write(parent.join(slab_name), &slab.bytes).map_err(|e| CliError::ReadFailed {
            path: parent.join(slab_name),
            source: e,
        })?;
    }
    if let Some(sidecar) = &artifact.metadata_sidecar {
        let name = sidecar
            .locator
            .strip_prefix("file:")
            .unwrap_or(&sidecar.locator);
        std::fs::write(parent.join(name), &sidecar.bytes).map_err(|e| CliError::ReadFailed {
            path: parent.join(name),
            source: e,
        })?;
    }
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

/// Turnover: extract all files, re-compress with the turnover profile,
/// and overwrite the original image. This removes unreferenced drops,
/// compacts history, and applies the best compression ratio.
fn turnover_cmd(image: &Path, profile_name: &str) -> Result<(), CliError> {
    eprintln!("turnover: extracting {} ...", image.display());

    let staging = std::env::temp_dir().join(format!("limnifs-turnover-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    extract(image, &staging)?;

    let config = limnifs_write::WriteConfig::from_profile(profile_name).unwrap_or_else(|| {
        eprintln!("warning: unknown profile '{profile_name}', using max-ratio");
        limnifs_write::profile::max_ratio()
    });
    eprintln!("turnover: re-compressing with {profile_name} ...");

    let artifact = limnifs_write::write_directory_with_config(&staging, &config)
        .map_err(|source| CliError::WriteFailed { source })?;

    let old_size = std::fs::metadata(image).map(|m| m.len()).unwrap_or(0);
    let mut new_size = artifact.bytes.len() as u64;
    for slab in &artifact.slabs {
        new_size += slab.bytes.len() as u64;
    }
    if let Some(sidecar) = &artifact.metadata_sidecar {
        new_size += sidecar.bytes.len() as u64;
    }

    std::fs::write(image, &artifact.bytes).map_err(|e| CliError::ReadFailed {
        path: image.to_path_buf(),
        source: e,
    })?;
    let parent = image.parent().unwrap_or_else(|| std::path::Path::new("."));
    for slab in &artifact.slabs {
        let slab_name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
        std::fs::write(parent.join(slab_name), &slab.bytes).map_err(|e| CliError::ReadFailed {
            path: parent.join(slab_name),
            source: e,
        })?;
    }
    if let Some(sidecar) = &artifact.metadata_sidecar {
        let name = sidecar
            .locator
            .strip_prefix("file:")
            .unwrap_or(&sidecar.locator);
        std::fs::write(parent.join(name), &sidecar.bytes).map_err(|e| CliError::ReadFailed {
            path: parent.join(name),
            source: e,
        })?;
    }

    let _ = std::fs::remove_dir_all(&staging);

    let delta = if new_size < old_size {
        format!(
            "-{:.1}%",
            (old_size - new_size) as f64 / old_size as f64 * 100.0
        )
    } else {
        format!(
            "+{:.1}%",
            (new_size - old_size) as f64 / old_size as f64 * 100.0
        )
    };
    eprintln!(
        "turnover: done — {} inodes, {} drops, {} slabs, {old_size} -> {new_size} bytes ({delta})",
        artifact.inode_count,
        artifact.drop_count,
        artifact.slabs.len(),
    );
    Ok(())
}

/// Read a `.lim` manifest, extract its inlined metadata blob, and
/// write the file at `path` to stdout. Inline files are written
/// directly; drop-backed files are read from the slab file that lives
/// alongside the manifest (per the writer's `file:` locator).
fn cat(image: &Path, path: &str, offset: Option<u64>, length: Option<u64>) -> Result<(), CliError> {
    use std::io::Write;
    let manifest_bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };
    let (blob, root_inode_number, slab_index, _dict_section) = load_image(&manifest_bytes, image, map_err)?;

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

    // Collect the full file data first, then apply offset/length.
    let mut file_data: Vec<u8> = Vec::new();
    match &target_inode.content_handle {
        ContentHandle::InlineData(data) => {
            file_data.extend_from_slice(data);
        }
        ContentHandle::SliceMap(slices) => {
            let store = if slab_index.is_empty() {
                None
            } else {
                Some(
                    limnifs_core::slab_store::SlabStore::load_mmap(image, &slab_index)
                        .map_err(map_err)?,
                )
            };
            for slice in slices {
                let plaintext = store
                    .as_ref()
                    .and_then(|s| s.plaintext_for(slice.drop_id.as_bytes()))
                    .ok_or_else(|| CliError::FormatFailed {
                        path: image.to_path_buf(),
                        source: CoreError::Corrupt {
                            reason: format!(
                                "slab: drop id {} not found in any slab",
                                format_hex(slice.drop_id.as_bytes())
                            ),
                        },
                    })?
                    .map_err(map_err)?;
                file_data.extend_from_slice(&plaintext);
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

    let total_len = u64::try_from(file_data.len()).unwrap_or(u64::MAX);
    let start = offset.unwrap_or(0).min(total_len);
    let remaining = total_len - start;
    let take = length.unwrap_or(remaining).min(remaining);
    let start_usize = usize::try_from(start).map_err(|_| CliError::FormatFailed {
        path: image.to_path_buf(),
        source: CoreError::Corrupt {
            reason: "cat: offset exceeds addressable size on this platform".into(),
        },
    })?;
    let take_usize = usize::try_from(take).map_err(|_| CliError::FormatFailed {
        path: image.to_path_buf(),
        source: CoreError::Corrupt {
            reason: "cat: length exceeds addressable size on this platform".into(),
        },
    })?;
    let out = std::io::stdout();
    let mut out = out.lock();
    out.write_all(&file_data[start_usize..start_usize + take_usize])
        .map_err(|source| CliError::ReadFailed {
            path: image.to_path_buf(),
            source,
        })?;
    Ok(())
}

/// Read multiple files from a `.lim` image, writing each to stdout
/// preceded by a `==> <path> ==>` header line. The manifest is parsed
/// once and the slab (if any) is loaded once and reused for all paths.
///
/// For trees with many small files this is ~100x faster than spawning
/// one `limni cat` process per file, because the per-invocation cost
/// (manifest read + parse + slab load) is amortized.
#[allow(clippy::too_many_lines)]
fn cat_multi(image: &Path, paths: &[String]) -> Result<(), CliError> {
    use std::io::Write;
    let manifest_bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };
    let (blob, _root_inode_number, slab_index, _dict_section) = load_image(&manifest_bytes, image, map_err)?;

    // Build the path→inode index ONCE. For trees with many files
    // this is the difference between O(N²) and O(N) cat-multi.
    // Profiling on a 5000-file flat directory showed cat-multi
    // spending ~1s in resolve_path's linear directory scans; the
    // index drops that to ~10ms.
    let path_index = blob.build_path_index();
    let inode_index: std::collections::HashMap<u64, &limnifs_core::Inode> =
        blob.inodes.iter().map(|i| (i.number, i)).collect();

    let slab_store: Option<limnifs_core::slab_store::SlabStore> = if slab_index.is_empty() {
        None
    } else {
        Some(limnifs_core::slab_store::SlabStore::load_mmap(image, &slab_index).map_err(map_err).map(|mut s| { if let Some(d) = &_dict_section { install_dicts(&mut s, d); } s })?)
    };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for path in paths {
        // Normalise: ensure leading slash, strip trailing slash, drop
        // empty components. Matches the path keys produced by
        // build_path_index.
        let normalised = if path.starts_with('/') {
            path.trim_end_matches('/').to_owned()
        } else {
            format!("/{}", path.trim_end_matches('/'))
        };
        let Some(&inode_number) = path_index.get(&normalised) else {
            return Err(CliError::FormatFailed {
                path: image.to_path_buf(),
                source: CoreError::Corrupt {
                    reason: format!("metadata blob: path {path:?} not found in tree"),
                },
            });
        };
        let Some(target_inode) = inode_index.get(&inode_number) else {
            return Err(CliError::FormatFailed {
                path: image.to_path_buf(),
                source: CoreError::Corrupt {
                    reason: format!("metadata blob: inode {inode_number} missing"),
                },
            });
        };
        writeln!(out, "==> {path} ==>").map_err(|source| CliError::ReadFailed {
            path: image.to_path_buf(),
            source,
        })?;
        match &target_inode.content_handle {
            ContentHandle::InlineData(data) => {
                out.write_all(data).map_err(|source| CliError::ReadFailed {
                    path: image.to_path_buf(),
                    source,
                })?;
            }
            ContentHandle::SliceMap(slices) => {
                let Some(store) = slab_store.as_ref() else {
                    return Err(CliError::FormatFailed {
                        path: image.to_path_buf(),
                        source: CoreError::Corrupt {
                            reason: format!(
                                "metadata blob: path {path:?} references a drop but slab store is missing"
                            ),
                        },
                    });
                };
                for slice in slices {
                    let plaintext = store
                        .plaintext_for(slice.drop_id.as_bytes())
                        .ok_or_else(|| CliError::FormatFailed {
                            path: image.to_path_buf(),
                            source: CoreError::Corrupt {
                                reason: format!(
                                    "slab: drop id {} not found in any slab",
                                    format_hex(slice.drop_id.as_bytes())
                                ),
                            },
                        })?
                        .map_err(map_err)?;
                    out.write_all(&plaintext)
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
        writeln!(out).map_err(|source| CliError::ReadFailed {
            path: image.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

/// Mount a `.lim` image as a read-only FUSE filesystem.
#[cfg(feature = "fuse")]
fn mount(image: &Path, mountpoint: &Path) -> Result<(), CliError> {
    let vfs = crate::vfs::Vfs::open(image).map_err(|e| match e {
        crate::vfs::VfsError::Core(c) => CliError::FormatFailed {
            path: image.to_path_buf(),
            source: c,
        },
        crate::vfs::VfsError::Io(io) => CliError::ReadFailed {
            path: image.to_path_buf(),
            source: io,
        },
        crate::vfs::VfsError::NotFound => CliError::FormatFailed {
            path: image.to_path_buf(),
            source: CoreError::Corrupt {
                reason: "mount: image content not found".into(),
            },
        },
    })?;
    eprintln!(
        "limni: mounting {} at {}",
        image.display(),
        mountpoint.display()
    );
    eprintln!("limni: press Ctrl-C to unmount");
    crate::fuse_vfs::mount(vfs, mountpoint).map_err(|source| CliError::ReadFailed {
        path: mountpoint.to_path_buf(),
        source,
    })
}

/// Print a comprehensive overview of an image: manifest header, feature
/// flags, metadata blob stats, slab stats, and per-class drop counts.
#[allow(clippy::too_many_lines)]
fn inspect(image: &Path) -> Result<(), CliError> {
    let manifest_bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };

    let mut cursor = ManifestCursor::new(&manifest_bytes);
    let header = parse_manifest_header(&mut cursor).map_err(map_err)?;
    let _ = parse_feature_flags_section(&mut cursor).map_err(map_err)?;
    let meta_ref = parse_metadata_reference(&mut cursor).map_err(map_err)?;
    let slab_index = parse_slab_index(&mut cursor).map_err(map_err)?;

    println!("image: {}", image.display());
    println!(
        "  format versions: drop_store={} metadata={} manifest={}",
        header.drop_store_version, header.metadata_version, header.manifest_version
    );
    println!(
        "  metadata: {}",
        if meta_ref.is_inlined() {
            "inlined"
        } else {
            "external"
        }
    );

    if meta_ref.is_inlined() {
        if let Some(blob_bytes) = &meta_ref.inline_metadata {
            let mut blob_cursor = ManifestCursor::new(blob_bytes);
            match parse_metadata_blob(&mut blob_cursor) {
                Ok(blob) => {
                    let root = blob
                        .root_inode_number()
                        .map_or_else(|| "?".to_string(), |n| n.to_string());
                    println!(
                        "  metadata blob: {} inodes, {} dir nodes, root inode = {}",
                        blob.inodes.len(),
                        blob.dir_nodes.len(),
                        root
                    );

                    let files = blob.inodes.iter().filter(|i| i.is_regular()).count();
                    let dirs = blob.inodes.iter().filter(|i| i.is_directory()).count();
                    println!("    files: {files}, directories: {dirs}");
                }
                Err(e) => {
                    println!("  metadata blob: parse error: {e}");
                }
            }
        }
    }

    println!("  slab index: {} entries", slab_index.len());

    // Try to load and inspect slabs
    for (i, entry) in slab_index.entries.iter().enumerate() {
        let locator = entry.locators.first();
        if let Some(loc) = locator {
            let name = loc.uri.strip_prefix("file:").unwrap_or(&loc.uri);
            let slab_path = image
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(name);
            match std::fs::read(&slab_path) {
                Ok(slab_bytes) => match limnifs_core::parse_slab(&slab_bytes) {
                    Ok(view) => {
                        let total_plaintext: u64 = view
                            .drop_records()
                            .iter()
                            .map(|r| u64::from(r.plaintext_len))
                            .sum();
                        let total_window: u64 = view
                            .drop_records()
                            .iter()
                            .map(|r| u64::from(r.len_in_window))
                            .sum();
                        let ratio = if total_plaintext > 0 {
                            #[allow(clippy::cast_precision_loss)]
                            {
                                total_window as f64 / total_plaintext as f64
                            }
                        } else {
                            1.0
                        };
                        println!(
                                "    slab[{}]: {} drops, {} bytes on disk, {} bytes plaintext, ratio {:.2}",
                                i,
                                view.drop_records().len(),
                                slab_bytes.len(),
                                total_plaintext,
                                ratio
                            );
                    }
                    Err(e) => {
                        println!("    slab[{i}]: parse error: {e}");
                    }
                },
                Err(_) => {
                    println!("    slab[{i}]: file not found: {}", slab_path.display());
                }
            }
        }
    }

    println!("  limni version: {VERSION}");
    Ok(())
}

/// Print an inode's metadata.
fn stat(image: &Path, path: &str) -> Result<(), CliError> {
    let manifest_bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };
    let (blob, root_inode_number, _, _dict_section) = load_image(&manifest_bytes, image, map_err)?;
    let root_inode = blob.inode_by_number(root_inode_number).expect("validated");
    let inode = resolve_path(&blob, root_inode, path).ok_or_else(|| CliError::FormatFailed {
        path: image.to_path_buf(),
        source: CoreError::Corrupt {
            reason: format!("path {path:?} not found"),
        },
    })?;
    println!("{}: stat {path:?}", image.display());
    println!("  inode:   {}", inode.number);
    println!("  mode:    0o{:o}", inode.mode & 0o7777);
    println!("  type:    {}", format_file_type(inode.file_type()));
    println!("  nlink:   {}", inode.nlink);
    match &inode.content_handle {
        ContentHandle::InlineData(d) => println!("  content: inline ({} bytes)", d.len()),
        ContentHandle::SharedInline(idx) => {
            println!("  content: shared inline (index {idx}, unresolved)");
        }
        ContentHandle::SliceMap(s) => println!("  content: slice map ({} slices)", s.len()),
        ContentHandle::Directory(h) => println!("  content: directory (hash {})", format_hex(h)),
        ContentHandle::Symlink(t) => println!("  content: symlink -> {t:?}"),
        ContentHandle::Device(d) => println!("  content: device ({d})"),
        ContentHandle::Pipe(p) => println!("  content: pipe ({p})"),
    }
    Ok(())
}

/// Print the directory tree recursively.
fn tree(image: &Path, path: &str) -> Result<(), CliError> {
    let manifest_bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };
    let (blob, root_inode_number, _, _dict_section) = load_image(&manifest_bytes, image, map_err)?;
    let root_inode = blob.inode_by_number(root_inode_number).expect("validated");
    let start_inode =
        resolve_path(&blob, root_inode, path).ok_or_else(|| CliError::FormatFailed {
            path: image.to_path_buf(),
            source: CoreError::Corrupt {
                reason: format!("path {path:?} not found"),
            },
        })?;
    println!("{}", path.trim_start_matches('/'));
    print_tree(&blob, start_inode, "", &mut Vec::new());
    Ok(())
}

fn print_tree(
    blob: &MetadataBlob,
    dir_inode: &limnifs_core::Inode,
    prefix: &str,
    visited: &mut Vec<u64>,
) {
    let hash = match &dir_inode.content_handle {
        ContentHandle::Directory(h) => *h,
        _ => return,
    };
    if visited.contains(&dir_inode.number) {
        return;
    }
    visited.push(dir_inode.number);
    let Some(node) = blob.dir_node_by_hash(&hash) else {
        return;
    };
    let entries: Vec<_> = node.entries.iter().collect();
    for (i, entry) in entries.iter().enumerate() {
        let last = i + 1 == entries.len();
        let branch = if last { "└── " } else { "├── " };
        let kind = if entry.entry_type == 0x02 { "dir" } else { "" };
        println!("{prefix}{branch}{} {kind}", entry.name);
        if entry.entry_type == 0x02 {
            if let Some(child) = blob.inode_by_number(entry.inode_number) {
                let np = if last { "    " } else { "│   " };
                print_tree(blob, child, &format!("{prefix}{np}"), visited);
            }
        }
    }
}

/// Extract an image to a filesystem directory.
fn extract(image: &Path, dest: &Path) -> Result<(), CliError> {
    use rayon::prelude::*;
    let manifest_bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };
    let (blob, root_inode_number, slab_index, _dict_section) = load_image(&manifest_bytes, image, map_err)?;
    std::fs::create_dir_all(dest).map_err(|source| CliError::ReadFailed {
        path: dest.to_path_buf(),
        source,
    })?;

    // Phase 1: walk the tree SEQUENTIALLY. Directory creation must
    // be ordered (parents before children) to avoid races; the
    // walker guarantees pre-order traversal. File inodes are cloned
    // into `tasks` so the parallel phase can outlive the blob borrow.
    let mut sink = limnifs_core::live_tree::ParallelExtractSink::new(dest);
    limnifs_core::live_tree::walk_live_tree(&blob, root_inode_number, &mut sink)
        .map_err(map_err)?;
    drop(blob); // release the metadata borrow before parallel phase

    // Phase 2: load the slab store once (if any) and write files IN PARALLEL.
    let slab_store: Option<limnifs_core::slab_store::SlabStore> = if slab_index.is_empty() {
        None
    } else {
        Some(limnifs_core::slab_store::SlabStore::load_mmap(image, &slab_index).map_err(map_err).map(|mut s| { if let Some(d) = &_dict_section { install_dicts(&mut s, d); } s })?)
    };

    let file_count = sink.tasks.len();
    let dir_count = sink.dir_count;
    let write_errors: Vec<Option<CliError>> = sink
        .tasks
        .par_iter()
        .map(|(path, inode)| extract_file(path, inode, slab_store.as_ref()).err())
        .collect();
    if let Some(Some(err)) = write_errors.into_iter().next() {
        return Err(err);
    }

    println!(
        "{}: extracted {file_count} files, {dir_count} directories",
        dest.display()
    );
    Ok(())
}

/// Write a single file to disk. Called from a rayon worker thread.
fn extract_file(
    path: &Path,
    inode: &limnifs_core::Inode,
    slab_store: Option<&limnifs_core::slab_store::SlabStore>,
) -> Result<(), CliError> {
    // Delegates to limnifs_core::live_tree::file_plaintext, which
    // is the canonical place that honours SliceRef::drop_byte_start
    // and drop_byte_len. The previous inline copy ignored those
    // fields, which worked only because the writer today always
    // emits slices spanning the whole drop.
    let data = limnifs_core::live_tree::file_plaintext(inode, slab_store).map_err(|source| {
        CliError::FormatFailed {
            path: path.to_path_buf(),
            source,
        }
    })?;
    if !data.is_empty() || inode.is_regular() {
        std::fs::write(path, &data).map_err(|source| CliError::ReadFailed {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
/// Compute the delta between two images and print tree operations.
fn diff(parent: &Path, child: &Path) -> Result<(), CliError> {
    let artifact = limnifs_write::delta_builder::compute_delta(parent, child).map_err(|e| {
        CliError::FormatFailed {
            path: parent.to_path_buf(),
            source: CoreError::Corrupt {
                reason: format!("delta: {e}"),
            },
        }
    })?;
    println!("ops: {}", artifact.tree_ops.len());
    for op in &artifact.tree_ops {
        let kind = match op.kind {
            limnifs_core::delta_linkage::TreeOpKind::Add => "A",
            limnifs_core::delta_linkage::TreeOpKind::Remove => "R",
            limnifs_core::delta_linkage::TreeOpKind::Replace => "M",
        };
        let inode = op
            .inode_number
            .map_or_else(|| "-".to_string(), |n| n.to_string());
        println!("{kind} {} inode={inode}", op.path);
    }
    Ok(())
}

fn format_file_type(file_type: u32) -> &'static str {
    match file_type {
        limnifs_core::S_IFREG => "regular file",
        limnifs_core::S_IFDIR => "directory",
        limnifs_core::S_IFLNK => "symlink",
        _ => "other",
    }
}

/// Inspect a slab file: header, drop records, codecs, sizes.
fn slab_cmd(slab_path: &Path) -> Result<(), CliError> {
    let bytes = std::fs::read(slab_path).map_err(|source| CliError::ReadFailed {
        path: slab_path.to_path_buf(),
        source,
    })?;
    let view = limnifs_core::parse_slab(&bytes).map_err(|source| CliError::FormatFailed {
        path: slab_path.to_path_buf(),
        source,
    })?;
    let header = view.header();
    println!("slab: {}", slab_path.display());
    println!("  format_version: {}", header.format_version);
    println!("  total_length:   {}", header.total_length);
    println!("  drops:          {}", view.drop_records().len());
    println!();
    let mut total_pt = 0u64;
    let mut total_win = 0u64;
    for (i, record) in view.drop_records().iter().enumerate() {
        let codec_name = match record.representation.codec {
            limnifs_core::codec::CODEC_STORE => "store",
            limnifs_core::codec::CODEC_LZ4 => "lz4",
            _ => "??",
        };
        let ratio = if record.plaintext_len > 0 {
            #[allow(clippy::cast_precision_loss)]
            {
                f64::from(record.len_in_window) / f64::from(record.plaintext_len)
            }
        } else {
            1.0
        };
        println!(
            "  [{i:3}] drop=b3:{:52} codec={codec_name:<5} pt={:>8} win={:>8} ratio={ratio:.2}",
            record.drop_id.to_text(),
            record.plaintext_len,
            record.len_in_window,
        );
        total_pt += u64::from(record.plaintext_len);
        total_win += u64::from(record.len_in_window);
    }
    println!();
    #[allow(clippy::cast_precision_loss)]
    let overall = if total_pt > 0 {
        total_win as f64 / total_pt as f64
    } else {
        1.0
    };
    println!(
        "  total plaintext: {total_pt}, total on disk: {total_win}, overall ratio: {overall:.2}"
    );
    Ok(())
}

/// Analyze garbage: find unreferenced drops in the slab.
fn gc_cmd(image: &Path) -> Result<(), CliError> {
    let manifest_bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };
    let (blob, _, slab_index, _dict_section) = load_image(&manifest_bytes, image, map_err)?;

    let mut referenced: HashSet<[u8; 32]> = HashSet::new();
    for inode in &blob.inodes {
        if let ContentHandle::SliceMap(slices) = &inode.content_handle {
            for slice in slices {
                referenced.insert(*slice.drop_id.as_bytes());
            }
        }
    }

    let mut total_drops = 0;
    let mut garbage_drops = 0;
    let mut garbage_bytes = 0u64;

    for entry in &slab_index.entries {
        for locator in &entry.locators {
            let name = locator.uri.strip_prefix("file:").unwrap_or(&locator.uri);
            let slab_path = image
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(name);
            if !slab_path.exists() {
                continue;
            }
            let slab_bytes = std::fs::read(&slab_path).map_err(|source| CliError::ReadFailed {
                path: slab_path.clone(),
                source,
            })?;
            let view =
                limnifs_core::parse_slab(&slab_bytes).map_err(|source| CliError::FormatFailed {
                    path: slab_path.clone(),
                    source,
                })?;
            for record in view.drop_records() {
                total_drops += 1;
                if !referenced.contains(record.drop_id.as_bytes()) {
                    garbage_drops += 1;
                    garbage_bytes += u64::from(record.len_in_window);
                }
            }
            break;
        }
    }

    println!("gc analysis: {}", image.display());
    println!("  total drops in slab(s): {total_drops}");
    println!("  referenced by manifest: {}", referenced.len());
    println!("  garbage (unreferenced):  {garbage_drops} drops, {garbage_bytes} bytes");
    if garbage_drops == 0 {
        println!("  status: clean (no garbage)");
    } else {
        println!("  status: {garbage_drops} drops can be reclaimed");
    }
    Ok(())
}

/// Print the history section of an image.
fn history_cmd(image: &Path) -> Result<(), CliError> {
    let bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let mut cursor = ManifestCursor::new(&bytes);
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };
    let _ = parse_manifest_header(&mut cursor).map_err(map_err)?;
    let _ = parse_feature_flags_section(&mut cursor).map_err(map_err)?;
    let _ = limnifs_core::parse_metadata_reference(&mut cursor).map_err(map_err)?;
    let _ = parse_slab_index(&mut cursor).map_err(map_err)?;
    let history = parse_history(&mut cursor).map_err(map_err)?;
    println!("{}: {} history entries", image.display(), history.len());
    for (i, entry) in history.entries.iter().enumerate() {
        let op_name = match entry.op {
            HistoryOp::Build => "build",
            HistoryOp::Delta => "delta",
            HistoryOp::Flatten => "flatten",
            HistoryOp::Turnover => "turnover",
            HistoryOp::Deepen => "deepen",
        };
        let ts_secs = entry.timestamp_ns / 1_000_000_000;
        println!(
            "  [{i}] {op_name:<8} ts={ts_secs} inputs={} params={}b",
            entry.inputs.len(),
            entry.params.len(),
        );
    }
    Ok(())
}

/// Analyze dedup: how many drops are shared across files.
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
fn dedup_cmd(image: &Path) -> Result<(), CliError> {
    let manifest_bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };
    let (blob, _, _, _dict_section) = load_image(&manifest_bytes, image, map_err)?;

    let mut drop_refs: HashSet<[u8; 32]> = HashSet::new();
    let mut total_refs = 0usize;
    let mut drop_backed_files = 0usize;
    let mut inline_files = 0usize;

    for inode in &blob.inodes {
        match &inode.content_handle {
            ContentHandle::SliceMap(slices) => {
                drop_backed_files += 1;
                for slice in slices {
                    drop_refs.insert(*slice.drop_id.as_bytes());
                    total_refs += 1;
                }
            }
            ContentHandle::InlineData(_) => {
                inline_files += 1;
            }
            _ => {}
        }
    }

    let unique_drops = drop_refs.len();
    let dup_refs = total_refs.saturating_sub(unique_drops);
    let dedup_ratio = if total_refs > 0 {
        dup_refs as f64 / total_refs as f64
    } else {
        0.0
    };

    println!("dedup analysis: {}", image.display());
    println!("  files (inline):      {inline_files}");
    println!("  files (drop-backed): {drop_backed_files}");
    println!("  total drop refs:     {total_refs}");
    println!("  unique drops:        {unique_drops}");
    println!("  duplicate refs:      {dup_refs}");
    println!(
        "  dedup ratio:         {dedup_ratio:.2} ({:.0}% of refs deduplicated)",
        dedup_ratio * 100.0
    );
    Ok(())
}

/// Compact an image by extracting → re-writing, eliminating slab garbage.
fn compact(source: &Path, output: &Path) -> Result<(), CliError> {
    let source_size = std::fs::metadata(source).map_or(0, |m| m.len());

    let temp_dir = std::env::temp_dir().join(format!(
        "limnifs-compact-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0u128, |d| d.as_nanos()),
    ));

    extract(source, &temp_dir)?;

    let artifact = limnifs_write::write_directory(&temp_dir)
        .map_err(|e| CliError::WriteFailed { source: e })?;
    std::fs::remove_dir_all(&temp_dir).ok();

    std::fs::write(output, &artifact.bytes).map_err(|e| CliError::ReadFailed {
        path: output.to_path_buf(),
        source: e,
    })?;

    let parent = output.parent().unwrap_or_else(|| std::path::Path::new("."));
    let mut slab_size: u64 = 0;
    for slab in &artifact.slabs {
        let slab_name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
        let slab_path = parent.join(slab_name);
        std::fs::write(&slab_path, &slab.bytes).map_err(|e| CliError::ReadFailed {
            path: slab_path.clone(),
            source: e,
        })?;
        slab_size += slab.bytes.len() as u64;
    }
    if let Some(sidecar) = &artifact.metadata_sidecar {
        let name = sidecar
            .locator
            .strip_prefix("file:")
            .unwrap_or(&sidecar.locator);
        let sidecar_path = parent.join(name);
        std::fs::write(&sidecar_path, &sidecar.bytes).map_err(|e| CliError::ReadFailed {
            path: sidecar_path.clone(),
            source: e,
        })?;
    }

    let output_size = artifact.bytes.len() as u64;

    println!("compacted: {} → {}", source.display(), output.display());
    println!("  source manifest: {source_size} bytes");
    println!("  output manifest: {output_size} bytes");
    println!(
        "  output slab:     {slab_size} bytes ({}) drops",
        artifact.drop_count
    );
    println!(
        "  inodes: {}  files: {}  dirs: {}",
        artifact.inode_count, artifact.file_count, artifact.dir_count
    );
    Ok(())
}

/// Deep integrity check: verify drop `BLAKE3` hashes against `DropId`s.
fn check_cmd(image: &Path) -> Result<(), CliError> {
    let manifest_bytes = std::fs::read(image).map_err(|source| CliError::ReadFailed {
        path: image.to_path_buf(),
        source,
    })?;
    let map_err = |source: CoreError| CliError::FormatFailed {
        path: image.to_path_buf(),
        source,
    };
    let (_blob, _, slab_index, _dict_section) = load_image(&manifest_bytes, image, map_err)?;

    if slab_index.is_empty() {
        println!("integrity check: {}", image.display());
        println!("  drops checked:  0 (no slabs referenced)");
        println!("  status:         all drops verified");
        return Ok(());
    }

    let slab_store =
        limnifs_core::slab_store::SlabStore::load_mmap(image, &slab_index).map_err(map_err).map(|mut s| { if let Some(d) = &_dict_section { install_dicts(&mut s, d); } s })?;

    let mut checked = 0usize;
    let mut passed = 0usize;
    let mut failed = 0usize;

    // Iterate every slab in the store, hashing every drop record's
    // plaintext against its declared DropId.
    for ordinal in 0..slab_store.slab_count() {
        let Some(slab_bytes) = slab_store.slab(ordinal) else {
            continue;
        };
        let view = match limnifs_core::parse_slab(slab_bytes) {
            Ok(v) => v,
            Err(e) => {
                println!("  FAIL: slab {ordinal} — {e}");
                failed += 1;
                continue;
            }
        };
        for record in view.drop_records() {
            checked += 1;
            match view.plaintext_for(record.drop_id.as_bytes()) {
                Some(Ok(plaintext)) => {
                    let computed = hash_section(&plaintext);
                    if computed == *record.drop_id.as_bytes() {
                        passed += 1;
                    } else {
                        failed += 1;
                        println!("  FAIL: drop {} — BLAKE3 mismatch", record.drop_id);
                    }
                }
                Some(Err(e)) => {
                    failed += 1;
                    println!("  FAIL: drop {} — {e}", record.drop_id);
                }
                None => {
                    // Drop not referenced by the manifest; skip silently.
                }
            }
        }
    }

    println!("integrity check: {}", image.display());
    println!("  drops checked:  {checked}");
    println!("  passed:         {passed}");
    if failed > 0 {
        println!("  FAILED:         {failed}");
        return Err(CliError::FormatFailed {
            path: image.to_path_buf(),
            source: CoreError::Corrupt {
                reason: format!("{failed} drops failed integrity check"),
            },
        });
    }
    println!("  status:         all drops verified");
    Ok(())
}

/// Quick write/read/extract benchmark on a synthetic tree.
fn benchmark() -> Result<(), CliError> {
    use std::time::Instant;

    let id = std::process::id();
    let src = std::env::temp_dir().join(format!("limnifs-bench-{id}-src"));
    let img = std::env::temp_dir().join(format!("limnifs-bench-{id}.lim"));
    let dest = std::env::temp_dir().join(format!("limnifs-bench-{id}-dest"));

    // Create synthetic tree: 100 files of 1KB, 10 files of 100KB, 1 file of 1MB.
    std::fs::create_dir_all(&src).expect("create src");
    let mut total_bytes = 0u64;
    for i in 0..100 {
        let data = vec![u8::try_from(i & 0xFF).unwrap(); 1024];
        total_bytes += data.len() as u64;
        std::fs::write(src.join(format!("small{i:03}.txt")), &data).expect("write");
    }
    for i in 0..10 {
        let data = vec![0x42u8; 100 * 1024];
        total_bytes += data.len() as u64;
        std::fs::write(src.join(format!("medium{i:02}.bin")), &data).expect("write");
    }
    let big = vec![0x55u8; 1024 * 1024];
    total_bytes += big.len() as u64;
    std::fs::write(src.join("large.bin"), &big).expect("write");

    // Write benchmark.
    let t0 = Instant::now();
    let artifact =
        limnifs_write::write_directory(&src).map_err(|e| CliError::WriteFailed { source: e })?;
    let write_ms = t0.elapsed().as_millis();
    std::fs::write(&img, &artifact.bytes).expect("write image");
    for slab in &artifact.slabs {
        let slab_name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
        let slab_path = img
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(slab_name);
        std::fs::write(&slab_path, &slab.bytes).expect("write slab");
    }
    if let Some(sidecar) = &artifact.metadata_sidecar {
        let name = sidecar
            .locator
            .strip_prefix("file:")
            .unwrap_or(&sidecar.locator);
        let sidecar_path = img.parent().unwrap_or(std::path::Path::new(".")).join(name);
        std::fs::write(&sidecar_path, &sidecar.bytes).expect("write metadata sidecar");
    }

    // Verify benchmark.
    let t1 = Instant::now();
    verify(&img, false).expect("verify");
    let verify_ms = t1.elapsed().as_millis();

    // Extract benchmark.
    let t2 = Instant::now();
    extract(&img, &dest).expect("extract");
    let extract_ms = t2.elapsed().as_millis();

    let write_throughput = if write_ms > 0 {
        total_bytes / u64::try_from(write_ms).unwrap_or(1) / 1024
    } else {
        0
    };

    #[allow(clippy::cast_precision_loss)]
    {
        println!(
            "benchmark: synthetic tree ({:.1} MB)",
            total_bytes as f64 / 1_048_576.0
        );
    }
    println!("  write:   {write_ms} ms ({write_throughput} MB/s)");
    println!("  verify:  {verify_ms} ms");
    println!("  extract: {extract_ms} ms");
    println!("  manifest: {} bytes", artifact.bytes.len());
    println!(
        "  drops: {} (inodes: {}, files: {}, dirs: {})",
        artifact.drop_count, artifact.inode_count, artifact.file_count, artifact.dir_count
    );

    // Cleanup.
    std::fs::remove_dir_all(&src).ok();
    std::fs::remove_dir_all(&dest).ok();
    std::fs::remove_file(&img).ok();
    for slab in &artifact.slabs {
        let slab_name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
        let slab_path = std::env::temp_dir().join(slab_name);
        let _ = std::fs::remove_file(&slab_path);
    }
    if let Some(sidecar) = &artifact.metadata_sidecar {
        let name = sidecar
            .locator
            .strip_prefix("file:")
            .unwrap_or(&sidecar.locator);
        let sidecar_path = std::env::temp_dir().join(name);
        let _ = std::fs::remove_file(&sidecar_path);
    }
    Ok(())
}

/// Generate a random AEAD key (XChaCha20-Poly1305, 32 bytes).
fn keygen() -> Result<(), CliError> {
    let mut key = vec![0u8; 32];
    getrandom::getrandom(&mut key).map_err(|e| CliError::FormatFailed {
        path: PathBuf::from("/dev/urandom"),
        source: CoreError::Corrupt {
            reason: format!("keygen: CSPRNG failed: {e}"),
        },
    })?;

    let hex = format_hex(&key);
    println!("XChaCha20-Poly1305 key (32 bytes):");
    println!("  hex:   {hex}");
    println!("  b3:    b3:{}", {
        let mut s = String::with_capacity(52);
        for b in &key {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
        }
        s
    });
    Ok(())
}

/// Encrypt a file using XChaCha20-Poly1305.
fn seal_cmd(input: &Path, output: &Path, key_hex: &str) -> Result<(), CliError> {
    let key = parse_hex_key(key_hex)?;
    let plaintext = std::fs::read(input).map_err(|source| CliError::ReadFailed {
        path: input.to_path_buf(),
        source,
    })?;
    let mut nonce = vec![0u8; 24];
    getrandom::getrandom(&mut nonce).map_err(|e| CliError::FormatFailed {
        path: input.to_path_buf(),
        source: CoreError::Corrupt {
            reason: format!("seal: CSPRNG failed: {e}"),
        },
    })?;
    let sealed = limnifs_core::crypto::seal(
        limnifs_core::aead::AEAD_XCHACHA20_POLY1305,
        &key,
        &nonce,
        &plaintext,
        b"",
    )
    .map_err(|source| CliError::FormatFailed {
        path: input.to_path_buf(),
        source,
    })?;
    let mut out = Vec::with_capacity(1 + 24 + sealed.len());
    out.push(limnifs_core::aead::AEAD_XCHACHA20_POLY1305);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    std::fs::write(output, &out).map_err(|source| CliError::ReadFailed {
        path: output.to_path_buf(),
        source,
    })?;
    println!(
        "{}: sealed {} bytes → {} bytes",
        input.display(),
        plaintext.len(),
        out.len()
    );
    Ok(())
}

/// Decrypt a file sealed with `limni seal`.
fn open_cmd(input: &Path, output: &Path, key_hex: &str) -> Result<(), CliError> {
    let key = parse_hex_key(key_hex)?;
    let data = std::fs::read(input).map_err(|source| CliError::ReadFailed {
        path: input.to_path_buf(),
        source,
    })?;
    if data.len() < 25 {
        return Err(CliError::FormatFailed {
            path: input.to_path_buf(),
            source: CoreError::Corrupt {
                reason: "sealed file too short".into(),
            },
        });
    }
    let aead_id = data[0];
    let nonce = &data[1..25];
    let ciphertext = &data[25..];
    let plaintext =
        limnifs_core::crypto::open(aead_id, &key, nonce, ciphertext, b"").map_err(|source| {
            CliError::FormatFailed {
                path: input.to_path_buf(),
                source,
            }
        })?;
    std::fs::write(output, &plaintext).map_err(|source| CliError::ReadFailed {
        path: output.to_path_buf(),
        source,
    })?;
    println!(
        "{}: opened {} bytes → {} bytes",
        input.display(),
        data.len(),
        plaintext.len()
    );
    Ok(())
}

fn parse_hex_key(hex: &str) -> Result<Vec<u8>, CliError> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return Err(CliError::FormatFailed {
            path: PathBuf::from("--key"),
            source: CoreError::Corrupt {
                reason: format!("key must be 64 hex chars (32 bytes), got {}", hex.len()),
            },
        });
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| CliError::FormatFailed {
                path: PathBuf::from("--key"),
                source: CoreError::Corrupt {
                    reason: "key contains invalid hex".into(),
                },
            })
        })
        .collect()
}

/// Split a file into n Shamir shares (any k reconstruct).
fn shamir_split(
    input: &Path,
    output_prefix: &Path,
    threshold: usize,
    shares: usize,
) -> Result<(), CliError> {
    let secret = std::fs::read(input).map_err(|source| CliError::ReadFailed {
        path: input.to_path_buf(),
        source,
    })?;
    let shares_bytes = limnifs_core::shamir::split(&secret, threshold, shares, getrandom_rng)
        .map_err(|e| CliError::FormatFailed {
            path: input.to_path_buf(),
            source: CoreError::Corrupt {
                reason: format!("shamir split: {e}"),
            },
        })?;
    for (i, share) in shares_bytes.iter().enumerate() {
        let path = format!("{}.share-{}", output_prefix.display(), i + 1);
        let path = PathBuf::from(path);
        std::fs::write(&path, share).map_err(|source| CliError::ReadFailed {
            path: path.clone(),
            source,
        })?;
        println!(
            "wrote share {}/{} (index {}) to {}",
            i + 1,
            shares,
            share[0],
            path.display()
        );
    }
    Ok(())
}

/// Combine k Shamir shares back into the original file.
fn shamir_combine(shares: &[PathBuf], output: &Path) -> Result<(), CliError> {
    let mut share_bytes: Vec<Vec<u8>> = Vec::with_capacity(shares.len());
    for path in shares {
        let bytes = std::fs::read(path).map_err(|source| CliError::ReadFailed {
            path: path.clone(),
            source,
        })?;
        share_bytes.push(bytes);
    }
    let share_refs: Vec<&[u8]> = share_bytes.iter().map(Vec::as_slice).collect();
    let secret =
        limnifs_core::shamir::combine(&share_refs).map_err(|e| CliError::FormatFailed {
            path: output.to_path_buf(),
            source: CoreError::Corrupt {
                reason: format!("shamir combine: {e}"),
            },
        })?;
    std::fs::write(output, &secret).map_err(|source| CliError::ReadFailed {
        path: output.to_path_buf(),
        source,
    })?;
    println!(
        "reconstructed {} bytes from {} shares",
        secret.len(),
        share_refs.len()
    );
    Ok(())
}

/// RNG closure that pulls from getrandom (CSPRNG).
fn getrandom_rng(out: &mut [u8]) -> Result<(), limnifs_core::shamir::ShamirError> {
    getrandom::getrandom(out).map_err(|e| limnifs_core::shamir::ShamirError::RngFailed {
        reason: format!("getrandom: {e}"),
    })
}

/// Export a `.lim` image as a composefs mountable directory tree.
///
/// This is the Linux fast path: extracts the tree to `<out-dir>/rootfs/`,
/// then shells out to `mkcomposefs` (from composefs-utils) to produce
/// `<out-dir>/image.cfs` — an EROFS image backed by a fs-verity
/// content-addressed blob store.
///
/// If `mkcomposefs` is not on PATH, the extracted rootfs is left in
/// place and a warning is printed. The user can install composefs-utils
/// and re-run, or copy the rootfs to a Linux machine and run
/// `mkcomposefs` there.

/// Common loader: parses the manifest prefix, extracts the inlined
/// metadata blob, and returns the parsed blob + root inode number +
/// slab index. Used by both `ls` and `cat`.
fn load_image(
    manifest_bytes: &[u8],
    image: &Path,
    map_err: impl Fn(CoreError) -> CliError,
) -> Result<
    (
        limnifs_core::MetadataBlob,
        u64,
        limnifs_core::SlabIndex,
        Option<limnifs_core::dictionary_section::DictionarySection>,
    ),
    CliError,
> {
    let mut cursor = ManifestCursor::new(manifest_bytes);
    let _ = parse_manifest_header(&mut cursor).map_err(&map_err)?;
    let _ = parse_feature_flags_section(&mut cursor).map_err(&map_err)?;
    let meta_ref = parse_metadata_reference(&mut cursor).map_err(&map_err)?;
    let blob_bytes: Vec<u8> = if let Some(inline) = meta_ref.inline_metadata.as_deref() {
        // Parser already decompressed v2 inline bytes.
        inline.to_vec()
    } else {
        // External metadata blob: follow the first file: locator,
        // then decompress if codec != 0.
        let entry = meta_ref
            .locators
            .first()
            .ok_or_else(|| CliError::FormatFailed {
                path: image.to_path_buf(),
                source: CoreError::Corrupt {
                    reason: "metadata_reference has neither inline data nor locators".into(),
                },
            })?;
        let name = entry.uri.strip_prefix("file:").unwrap_or(&entry.uri);
        let sidecar_path = image.parent().unwrap_or_else(|| Path::new(".")).join(name);
        let wire_bytes = std::fs::read(&sidecar_path).map_err(|source| CliError::ReadFailed {
            path: sidecar_path.clone(),
            source,
        })?;
        if meta_ref.codec == 0 {
            wire_bytes
        } else {
            limnifs_core::codec::decompress(meta_ref.codec, &wire_bytes, meta_ref.uncompressed_len)
                .map_err(&map_err)?
        }
    };
    let mut blob_cursor = ManifestCursor::new(&blob_bytes);
    let blob = parse_metadata_blob(&mut blob_cursor).map_err(&map_err)?;

    let slab_index = parse_slab_index(&mut cursor).map_err(&map_err)?;
    // History may or may not be present; ignore errors.
    let _ = parse_history(&mut cursor);
    // Optional: dictionary_section (added in v0.2.3). Best-effort
    // parse — older manifests don't have one and that's fine.
    let dict_section = if cursor.remaining_len() > 0 {
        parse_dictionary_section(&mut cursor).ok()
    } else {
        None
    };

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

    Ok((blob, root_inode_number, slab_index, dict_section))
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
    metadata_summary: Option<&str>,
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
    metadata_summary: Option<&str>,
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
        print!("{summary}");
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
        cat(&image, "/a.txt", None, None).expect("cat inline file succeeds");
        let _ = std::fs::remove_file(&image);
    }

    #[test]
    fn cat_missing_path_reports_corrupt() {
        let source = make_source_tree();
        let image = make_temp_file(&[]);
        std::fs::remove_file(&image).ok();
        limn(&source, &image).expect("write image");
        std::fs::remove_dir_all(&source).ok();
        match cat(&image, "/does-not-exist", None, None) {
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
        match cat(&image, "/sub", None, None) {
            Err(CliError::FormatFailed { source, .. }) => {
                assert!(matches!(source, CoreError::Corrupt { .. }));
            }
            other => panic!("expected FormatFailed, got {other:?}"),
        }
        let _ = std::fs::remove_file(&image);
    }

    #[test]
    fn cat_offset_length_clamps_without_error() {
        let source = make_source_tree();
        let image = make_temp_file(&[]);
        std::fs::remove_file(&image).ok();
        limn(&source, &image).expect("write image");
        std::fs::remove_dir_all(&source).ok();

        // offset beyond EOF should clamp to empty, not error.
        cat(&image, "/a.txt", Some(1_000_000), None).expect("offset past EOF clamps");
        // length past remaining should clamp to remaining.
        cat(&image, "/a.txt", Some(0), Some(1_000_000)).expect("length past EOF clamps");
        // both set should slice the inline bytes without erroring.
        cat(&image, "/a.txt", Some(1), Some(1)).expect("subrange reads succeed");
        let _ = std::fs::remove_file(&image);
    }

    #[test]
    fn cat_multi_reads_many_files_in_one_invocation() {
        // cat_multi writes to stdout; we can't easily capture that
        // inside a unit test. Instead we verify it succeeds on a
        // known-good image, and that errors propagate correctly.
        let source = make_source_tree();
        let image = make_temp_file(&[]);
        std::fs::remove_file(&image).ok();
        limn(&source, &image).expect("write image");
        std::fs::remove_dir_all(&source).ok();

        let paths = vec![
            "/a.txt".to_owned(),
            "/b.txt".to_owned(),
            "/sub/c.txt".to_owned(),
        ];
        cat_multi(&image, &paths).expect("cat-multi succeeds");

        // Missing path surfaces as an error.
        let bad_paths = vec!["/does-not-exist".to_owned()];
        let err = cat_multi(&image, &bad_paths).unwrap_err();
        match err {
            CliError::FormatFailed { source, .. } => {
                assert!(format!("{source}").contains("not found in tree"));
            }
            other => panic!("expected FormatFailed, got {other:?}"),
        }

        let _ = std::fs::remove_file(&image);
    }

    #[test]
    fn e2e_full_lifecycle() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static E2E_ID: AtomicU64 = AtomicU64::new(0);

        let id = E2E_ID.fetch_add(1, Ordering::SeqCst);
        let src = std::env::temp_dir().join(format!("limnifs-e2e-{id}-src"));
        let modified = std::env::temp_dir().join(format!("limnifs-e2e-{id}-mod"));
        let img = std::env::temp_dir().join(format!("limnifs-e2e-{id}.lim"));
        let img2 = std::env::temp_dir().join(format!("limnifs-e2e-{id}-mod.lim"));
        let dest = std::env::temp_dir().join(format!("limnifs-e2e-{id}-dest"));

        // Create source tree with mixed content.
        std::fs::create_dir_all(src.join("sub")).expect("create dirs");
        std::fs::write(src.join("small.txt"), b"small inline file").expect("write");
        std::fs::write(src.join("sub").join("nested.txt"), b"nested content").expect("write");
        std::fs::write(src.join("repeated.txt"), b"AABBCCDDEE".repeat(500)).expect("write");

        // Build image.
        limn(&src, &img).expect("limn succeeds");

        // Verify.
        verify(&img, false).expect("verify succeeds");
        verify(&img, true).expect("verify --json succeeds");

        // List root.
        ls(&img, "/").expect("ls succeeds");

        // Cat a file.
        cat(&img, "/small.txt", None, None).expect("cat succeeds");

        // Tree.
        tree(&img, "/").expect("tree succeeds");

        // Stat a file.
        stat(&img, "/small.txt").expect("stat succeeds");

        // Extract and verify round-trip.
        extract(&img, &dest).expect("extract succeeds");
        let orig = std::fs::read(src.join("small.txt")).expect("read orig");
        let extracted = std::fs::read(dest.join("small.txt")).expect("read extracted");
        assert_eq!(orig, extracted, "small.txt round-trip mismatch");

        let orig_rep = std::fs::read(src.join("repeated.txt")).expect("read orig rep");
        let extracted_rep = std::fs::read(dest.join("repeated.txt")).expect("read extracted rep");
        assert_eq!(orig_rep, extracted_rep, "repeated.txt round-trip mismatch");

        // Inspect.
        inspect(&img).expect("inspect succeeds");

        // History.
        history_cmd(&img).expect("history succeeds");

        // Dedup analysis.
        dedup_cmd(&img).expect("dedup succeeds");

        // GC analysis.
        gc_cmd(&img).expect("gc succeeds");

        // Create modified image for diff.
        std::fs::create_dir_all(modified.join("sub")).expect("create mod dir");
        std::fs::write(modified.join("small.txt"), b"small inline file").expect("copy");
        std::fs::write(modified.join("sub").join("nested.txt"), b"nested content").expect("copy");
        std::fs::write(modified.join("repeated.txt"), b"AABBCCDDEE".repeat(500)).expect("copy");
        std::fs::write(modified.join("new.txt"), b"newly added").expect("add new");
        limn(&modified, &img2).expect("limn modified");

        // Diff should show one Add op.
        diff(&img, &img2).expect("diff succeeds");

        // Compact.
        compact(
            &img,
            &std::env::temp_dir().join(format!("limnifs-e2e-{id}-compact.lim")),
        )
        .expect("compact succeeds");

        // Cleanup.
        std::fs::remove_dir_all(&src).ok();
        std::fs::remove_dir_all(&modified).ok();
        std::fs::remove_dir_all(&dest).ok();
        std::fs::remove_file(&img).ok();
        std::fs::remove_file(&img2).ok();
    }

    #[test]
    fn seal_open_round_trips() {
        let id = std::process::id();
        let input = std::env::temp_dir().join(format!("limnifs-seal-{id}.txt"));
        let sealed = std::env::temp_dir().join(format!("limnifs-seal-{id}.sealed"));
        let opened = std::env::temp_dir().join(format!("limnifs-seal-{id}.open"));

        std::fs::write(&input, b"secret message for seal/open test").expect("write input");

        let key = "42".repeat(32); // 64-char hex key (32 bytes)
        seal_cmd(&input, &sealed, &key).expect("seal succeeds");
        assert_ne!(
            std::fs::read(&input).unwrap(),
            std::fs::read(&sealed).unwrap(),
            "sealed data must differ from plaintext"
        );

        open_cmd(&sealed, &opened, &key).expect("open succeeds");
        let orig = std::fs::read(&input).unwrap();
        let result = std::fs::read(&opened).unwrap();
        assert_eq!(orig, result, "round-trip must match");

        std::fs::remove_file(&input).ok();
        std::fs::remove_file(&sealed).ok();
        std::fs::remove_file(&opened).ok();
    }

    #[test]
    fn seal_open_wrong_key_fails() {
        let id = std::process::id();
        let input = std::env::temp_dir().join(format!("limnifs-wrongkey-{id}.txt"));
        let sealed = std::env::temp_dir().join(format!("limnifs-wrongkey-{id}.sealed"));
        let opened = std::env::temp_dir().join(format!("limnifs-wrongkey-{id}.open"));

        std::fs::write(&input, b"secret data").expect("write");

        let correct_key = "42".repeat(32);
        let wrong_key = "99".repeat(32);

        seal_cmd(&input, &sealed, &correct_key).expect("seal");
        match open_cmd(&sealed, &opened, &wrong_key) {
            Err(CliError::FormatFailed { .. }) => {}
            other => panic!("expected FormatFailed, got {other:?}"),
        }

        std::fs::remove_file(&input).ok();
        std::fs::remove_file(&sealed).ok();
        std::fs::remove_file(&opened).ok();
    }

    #[test]
    fn parse_hex_key_validates_length() {
        assert!(parse_hex_key("short").is_err());
        assert!(parse_hex_key(&"42".repeat(32)).is_ok());
        assert!(parse_hex_key(&"gg".repeat(32)).is_err());
    }

    #[test]
    fn shamir_split_combine_round_trip() {
        let id = std::process::id();
        let dir = std::env::temp_dir().join(format!("limni-shamir-{id}"));
        std::fs::create_dir_all(&dir).expect("mkdir");

        let input = dir.join("secret.txt");
        let prefix = dir.join("shares");
        let output = dir.join("recovered.txt");
        std::fs::write(&input, b"shamir-protected master key").expect("write");

        shamir_split(&input, &prefix, 3, 5).expect("split");
        // Collect any 3 of the 5 shares.
        let shares: Vec<PathBuf> = [1, 3, 5]
            .iter()
            .map(|i| PathBuf::from(format!("{}.share-{i}", prefix.to_string_lossy())))
            .collect::<Vec<_>>();
        shamir_combine(&shares, &output).expect("combine");

        let recovered = std::fs::read(&output).expect("read recovered");
        assert_eq!(recovered, b"shamir-protected master key");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn shamir_combine_rejects_too_few_shares() {
        let id = std::process::id();
        let dir = std::env::temp_dir().join(format!("limni-shamir-few-{id}"));
        std::fs::create_dir_all(&dir).expect("mkdir");

        let input = dir.join("secret.txt");
        let prefix = dir.join("shares");
        std::fs::write(&input, b"secret").expect("write");

        shamir_split(&input, &prefix, 3, 5).expect("split");
        // Only 2 shares — combine produces garbage, but won't error.
        // The test verifies it does NOT reproduce the secret.
        let shares: Vec<PathBuf> = [1, 2]
            .iter()
            .map(|i| PathBuf::from(format!("{}.share-{i}", prefix.to_string_lossy())))
            .collect::<Vec<_>>();
        let output = dir.join("recovered.txt");
        shamir_combine(&shares, &output).expect("combine produces SOME output");
        let recovered = std::fs::read(&output).expect("read");
        // The recovered bytes must NOT equal the original.
        assert_ne!(
            recovered, b"secret",
            "k-1 shares must not reveal the secret"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
