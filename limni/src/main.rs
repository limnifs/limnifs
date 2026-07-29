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
use limnifs_core::{parse_manifest_header, CoreError, ManifestHeader, MANIFEST_HEADER_LEN};

const VERSION: &str = env!("CARGO_PKG_VERSION");

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
    /// Validate a manifest header.
    ///
    /// Reads the first 16 bytes of the target file and checks the magic,
    /// the three per-layer version numbers, and the reserved-zero field
    /// (spec §5.1). Does not yet verify the Merkle root or AEAD tags —
    /// that arrives with component 03-core-reader.
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
    let mut header_buf = [0u8; MANIFEST_HEADER_LEN];
    let read_len = file
        .read(&mut header_buf)
        .map_err(|source| CliError::ReadFailed {
            path: image.clone(),
            source,
        })?;
    let header = parse_manifest_header(&header_buf[..read_len]).map_err(|source| {
        CliError::FormatFailed {
            path: image.clone(),
            source,
        }
    })?;
    print_report(image, header, json);
    Ok(())
}

fn print_report(path: &Path, header: ManifestHeader, json: bool) {
    if json {
        println!(
            "{{\"path\":\"{}\",\"magic\":\"LMFS\",\"drop_store_version\":{},\"metadata_version\":{},\"manifest_version\":{}}}",
            escape_json_path(path),
            header.drop_store_version,
            header.metadata_version,
            header.manifest_version
        );
    } else {
        println!("{}: valid LimniFS manifest header", path.display());
        println!("  magic:               LMFS");
        println!("  drop store version:  {}", header.drop_store_version);
        println!("  metadata version:    {}", header.metadata_version);
        println!("  manifest version:    {}", header.manifest_version);
        println!("  limni version:       {VERSION}");
    }
}

fn escape_json_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_temp_file(contents: &[u8]) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "limni-test-{}-{}.lim",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
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
    fn verify_rejects_short_file() {
        let path = make_temp_file(&[0u8; 10]);
        match verify(&path, false) {
            Err(CliError::FormatFailed { source, .. }) => {
                assert!(matches!(source, CoreError::TooShort { .. }));
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
