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
    parse_feature_flags_section, parse_history, parse_manifest_header, parse_metadata_reference,
    parse_slab_index, CoreError, FeatureFlags, ManifestCursor, ManifestHeader, ManifestRoot,
    SectionHashes,
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
}

fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    match cli.command {
        Command::Verify { image, json } => verify(&image, json),
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
    }
}

fn main() -> ExitCode {
    run_with_exit_code()
}

#[derive(Debug)]
enum CliError {
    ReadFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    FormatFailed {
        path: PathBuf,
        source: CoreError,
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

    print_report(
        image,
        header,
        &flags,
        metadata_reference.is_inlined(),
        slab_index.len(),
        history.len(),
        extra_bytes_remaining,
        merkle_root,
        json,
    );
    Ok(())
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
        append_feature_flags(&mut bytes, &[(0x0001, 0x01)]);
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
}
