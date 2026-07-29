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
    parse_feature_flags_section, parse_manifest_header, CoreError, FeatureFlags, ManifestCursor,
    ManifestHeader,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Read at most this many bytes when verifying. Covers the 16-byte
/// header plus a generous feature-flags section; the verify command
/// does not need the whole manifest.
const VERIFY_READ_BUDGET: usize = 4096;

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
    /// Validate a manifest header (and the feature flags section when present).
    ///
    /// Reads up to the first 4 KB of the target file, parses the
    /// manifest header (spec §5.1), and parses the feature flags
    /// section (§5.2) when bytes remain. Full Merkle-root and AEAD
    /// verification arrives with component 03-core-reader.
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
    let header = parse_manifest_header(&mut cursor).map_err(|source| CliError::FormatFailed {
        path: image.clone(),
        source,
    })?;
    let flags = if cursor.remaining_len() >= 1 {
        Some(
            parse_feature_flags_section(&mut cursor).map_err(|source| CliError::FormatFailed {
                path: image.clone(),
                source,
            })?,
        )
    } else {
        None
    };
    print_report(image, header, flags.as_ref(), json);
    Ok(())
}

fn print_report(path: &Path, header: ManifestHeader, flags: Option<&FeatureFlags>, json: bool) {
    if json {
        print_json_report(path, header, flags);
    } else {
        print_human_report(path, header, flags);
    }
}

fn print_human_report(path: &Path, header: ManifestHeader, flags: Option<&FeatureFlags>) {
    println!("{}: valid LimniFS manifest header", path.display());
    println!("  magic:               LMFS");
    println!("  drop store version:  {}", header.drop_store_version);
    println!("  metadata version:    {}", header.metadata_version);
    println!("  manifest version:    {}", header.manifest_version);
    match flags {
        None => println!("  feature flags:       (section absent)"),
        Some(flags) if flags.is_empty() => println!("  feature flags:       0 entries"),
        Some(flags) => {
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
    }
    println!("  limni version:       {VERSION}");
}

fn print_json_report(path: &Path, header: ManifestHeader, flags: Option<&FeatureFlags>) {
    let escaped_path = escape_json_path(path);
    print!("{{\"path\":\"{escaped_path}\",\"magic\":\"LMFS\",");
    print!("\"drop_store_version\":{},", header.drop_store_version);
    print!("\"metadata_version\":{},", header.metadata_version);
    print!("\"manifest_version\":{}", header.manifest_version);
    match flags {
        None => print!(",\"feature_flags\":null"),
        Some(flags) => {
            print!(",\"feature_flags\":[");
            for (i, entry) in flags.entries.iter().enumerate() {
                if i > 0 {
                    print!(",");
                }
                let required = if entry.required { "true" } else { "false" };
                print!("{{\"flag_id\":{},\"required\":{required}}}", entry.flag_id);
            }
            print!("]");
        }
    }
    println!("}}");
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

    #[test]
    fn verify_accepts_current_header() {
        let path = make_temp_file(&make_current_header());
        let result = verify(&path, false);
        assert!(result.is_ok(), "{result:?}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verify_parses_header_plus_empty_feature_flags() {
        let mut bytes = Vec::from(make_current_header());
        bytes.push(0x01); // feature flags section version 1
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 0 entries
        let path = make_temp_file(&bytes);
        verify(&path, false).expect("header + empty flags parse");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verify_parses_header_plus_one_required_flag() {
        let mut bytes = Vec::from(make_current_header());
        bytes.push(0x01); // section version 1
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 entry
        bytes.extend_from_slice(&0x0001u16.to_le_bytes()); // EC
        bytes.push(0x01); // required
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
        let path = make_temp_file(&make_current_header());
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
