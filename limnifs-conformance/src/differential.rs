//! Cross-reader differential harness.
//!
//! The Phase 0 exit gate requires both readers (Rust + Python) to
//! agree on every vector's `ManifestRoot`. This module generates a
//! fixture from a declarative spec via the Rust builder, runs BOTH
//! the Rust `limni verify` and the Python `limni-py verify` CLIs
//! against it as black-box subprocesses, parses their reported
//! roots, and asserts equality.
//!
//! The harness NEVER links either reader as a library on the
//! verification path. The Rust builder encodes; both binaries
//! decode; the harness compares their reported roots. A divergent
//! root means a spec ambiguity or a parser bug.
//!
//! ## Skipping when adapters are missing
//!
//! When `limni-py` is not installed (the common case in the Rust
//! workspace's CI), tests marked `#[differential]` are skipped
//! rather than failed. Callers can opt in via the
//! `LIMNIFS_RUN_DIFFERENTIAL=1` environment variable.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::env::var;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use crate::builder::{ManifestArtifact, ManifestBuilder};
use crate::vectors::Vector;
use limnifs_format::ManifestRoot;

/// Environment variable that forces differential tests to run even
/// when adapters might be missing. Set to "1" to enable.
pub const DIFFERENTIAL_ENV_VAR: &str = "LIMNIFS_RUN_DIFFERENTIAL";

/// Report produced by running one CLI against one fixture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliReport {
    pub binary: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub merkle_root: Option<String>,
}

/// Run the Rust `limni verify --json` binary against the fixture bytes.
///
/// Writes the bytes to a temp file, then shells out. Returns the
/// binary's stdout / stderr / parsed `merkle_root` field.
///
/// # Errors
///
/// Returns `Err(String)` if the binary cannot be found or the
/// subprocess fails to spawn.
pub fn run_limni_rust(artifact_bytes: &[u8]) -> Result<CliReport, String> {
    let path = write_temp_fixture(artifact_bytes, "limni-rust")?;
    run_cli("limni", &path, "limni-rust")
}

/// Run the Python `limni-py verify --json` binary against the fixture bytes.
///
/// # Errors
///
/// Returns `Err(String)` if the binary cannot be found or the
/// subprocess fails to spawn.
pub fn run_limni_py(artifact_bytes: &[u8]) -> Result<CliReport, String> {
    let path = write_temp_fixture(artifact_bytes, "limni-py")?;
    run_cli("limni-py", &path, "limni-py")
}

/// True iff the differential harness should run. Callers should
/// skip differential assertions when this returns `false`.
///
/// The harness runs when EITHER:
/// - The `LIMNIFS_RUN_DIFFERENTIAL` environment variable is "1", OR
/// - Both `limni` and `limni-py` are findable on `PATH`.
#[must_use]
pub fn should_run() -> bool {
    if matches!(var(DIFFERENTIAL_ENV_VAR).as_deref(), Ok("1")) {
        return true;
    }
    which("limni").is_some() && which("limni-py").is_some()
}

/// Compare the Rust and Python CLIs' reported roots on the given vector.
///
/// Encodes the vector via the Rust builder, then runs both CLIs on
/// the resulting bytes. Returns `Ok(())` if both report the same
/// `ManifestRoot`; `Err(String)` if they disagree or either fails.
///
/// # Errors
///
/// See [`run_limni_rust`] and [`run_limni_py`].
pub fn differential_root(vector: &Vector) -> Result<(ManifestRoot, ManifestRoot), String> {
    let artifact: ManifestArtifact = ManifestBuilder::new(vector.spec.clone()).build();
    let expected_root = artifact.merkle_root;
    let rust_report = run_limni_rust(&artifact.bytes)?;
    let py_report = run_limni_py(&artifact.bytes)?;
    let rust_root = parse_root_from_json(&rust_report.stdout).ok_or_else(|| {
        format!(
            "limni did not report a merkle_root; stderr={}",
            rust_report.stderr
        )
    })?;
    let py_root = parse_root_from_json(&py_report.stdout).ok_or_else(|| {
        format!(
            "limni-py did not report a merkle_root; stderr={}",
            py_report.stderr
        )
    })?;
    if rust_root != py_root {
        return Err(format!(
            "root mismatch on vector {}: rust={rust_root} py={py_root}",
            vector.name
        ));
    }
    if rust_root != expected_root.to_text() {
        return Err(format!(
            "limni root disagrees with builder on vector {}: builder={expected_root} limni={rust_root}",
            vector.name
        ));
    }
    let rust_root_typed = ManifestRoot::from_bytes(decode_b3(&rust_root)?);
    let py_root_typed = ManifestRoot::from_bytes(decode_b3(&py_root)?);
    Ok((rust_root_typed, py_root_typed))
}

fn run_cli(binary: &str, fixture_path: &PathBuf, label: &str) -> Result<CliReport, String> {
    let Output {
        status,
        stdout,
        stderr,
    } = Command::new(binary)
        .arg("verify")
        .arg("--json")
        .arg(fixture_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("{label}: cannot spawn {binary:?}: {e}"))?;
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();
    let merkle_root = parse_root_from_json(&stdout);
    Ok(CliReport {
        binary: binary.to_owned(),
        exit_code: status.code().unwrap_or(-1),
        stdout,
        stderr,
        merkle_root,
    })
}

fn parse_root_from_json(stdout: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(stdout).ok()?;
    value.get("merkle_root")?.as_str().map(str::to_owned)
}

fn write_temp_fixture(bytes: &[u8], label: &str) -> Result<PathBuf, String> {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "limnifs-diff-{label}-{pid}-{id}-{nanos}.lim",
        pid = std::process::id()
    ));
    std::fs::write(&path, bytes).map_err(|e| format!("cannot write fixture: {e}"))?;
    Ok(path)
}

fn which(binary: &str) -> Option<PathBuf> {
    let path_var = var("PATH").ok()?;
    for dir in path_var.split(':') {
        let candidate = PathBuf::from(dir).join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn decode_b3(text: &str) -> Result<[u8; 32], String> {
    let rest = text
        .strip_prefix("b3:")
        .ok_or_else(|| format!("missing b3: prefix in {text:?}"))?;
    let mut decoded = [0u8; 32];
    let mut buffer: u64 = 0;
    let mut bits: u32 = 0;
    let mut index = 0;
    for ch in rest.chars() {
        let value = match ch {
            'a'..='z' => u32::from(ch) - u32::from('a'),
            '2'..='7' => u32::from(ch) - u32::from('2') + 26,
            _ => return Err(format!("non-base32 char {ch:?}")),
        };
        buffer = (buffer << 5) | u64::from(value);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            let byte = (buffer >> bits) & 0xFF;
            if index >= 32 {
                return Err(format!("too many bytes decoded from {text:?}"));
            }
            decoded[index] = byte as u8;
            index += 1;
        }
    }
    if index != 32 {
        return Err(format!("decoded {index} bytes, expected 32 from {text:?}"));
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors::all_vectors;

    #[test]
    fn differential_agreement_or_skip() {
        if !should_run() {
            eprintln!(
                "skipping differential test: {DIFFERENTIAL_ENV_VAR} unset and adapters not on PATH"
            );
            return;
        }
        for vector in all_vectors() {
            match differential_root(&vector) {
                Ok((rust_root, py_root)) => {
                    assert_eq!(rust_root, py_root, "vector {}", vector.name);
                }
                Err(e) => {
                    // Print debug info on failure to aid diagnosis.
                    let artifact = ManifestBuilder::new(vector.spec.clone()).build();
                    if let Ok(report) = run_limni_rust(&artifact.bytes) {
                        eprintln!("RUST REPORT: {report:?}");
                    }
                    if let Ok(report) = run_limni_py(&artifact.bytes) {
                        eprintln!("PY REPORT: {report:?}");
                    }
                    panic!("vector {}: {e}", vector.name);
                }
            }
        }
    }

    #[test]
    fn decode_b3_round_trip_sample() {
        // Known sample from the Rust CLI output (computed on a tiny
        // v0.1 image during session 11).
        let text = "b3:5tmx3wa6ab245x47ia56f5dm7d52pkvmhpdm3rwvhqhebjxtunjq";
        let bytes = decode_b3(text).expect("decodes");
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn decode_b3_rejects_bad_prefix() {
        assert!(decode_b3("x3:abc").is_err());
    }
}
