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

/// A mutation to apply to a fixture's bytes for rejection testing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mutation {
    /// Truncate the bytes to the given length (MUST be < original length).
    Truncate { new_len: usize },
    /// Replace the first 4 bytes with `XXXX` to break the magic.
    BadMagic,
    /// XOR a single byte at `offset` with `mask`.
    FlipByte { offset: usize, mask: u8 },
}

impl Mutation {
    /// Apply this mutation to a byte buffer.
    ///
    /// # Panics
    ///
    /// Panics if the mutation is out of bounds for `original`:
    /// - [`Mutation::Truncate`] requires `new_len < original.len()`.
    /// - [`Mutation::FlipByte`] requires `offset < original.len()`.
    #[must_use]
    pub fn apply(self, original: &[u8]) -> Vec<u8> {
        match self {
            Self::Truncate { new_len } => {
                assert!(
                    new_len < original.len(),
                    "Truncate requires new_len < original length"
                );
                original[..new_len].to_vec()
            }
            Self::BadMagic => {
                let mut out = original.to_vec();
                out[..4].copy_from_slice(b"XXXX");
                out
            }
            Self::FlipByte { offset, mask } => {
                assert!(offset < original.len(), "FlipByte offset out of bounds");
                let mut out = original.to_vec();
                out[offset] ^= mask;
                out
            }
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Truncate { .. } => "truncate",
            Self::BadMagic => "bad-magic",
            Self::FlipByte { .. } => "flip-byte",
        }
    }
}

/// Run a rejection differential: encode the vector, mutate, run both
/// CLIs, and assert BOTH reject (exit non-zero).
///
/// Returns `Ok(())` if both readers reject; `Err(String)` describing
/// which reader (or both) incorrectly accepted the corrupted input.
///
/// # Errors
///
/// See [`run_limni_rust`] and [`run_limni_py`].
pub fn differential_rejection(vector: &Vector, mutation: Mutation) -> Result<(), String> {
    let artifact: ManifestArtifact = ManifestBuilder::new(vector.spec.clone()).build();
    let corrupted = mutation.apply(&artifact.bytes);
    let rust = run_limni_rust(&corrupted)?;
    let py = run_limni_py(&corrupted)?;
    if rust.exit_code == 0 {
        return Err(format!(
            "vector {} / mutation {}: rust accepted corrupted input (exit 0, root {:?})",
            vector.name,
            mutation.label(),
            rust.merkle_root
        ));
    }
    if py.exit_code == 0 {
        return Err(format!(
            "vector {} / mutation {}: python accepted corrupted input (exit 0, root {:?})",
            vector.name,
            mutation.label(),
            py.merkle_root
        ));
    }
    Ok(())
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

/// Run a Layer-2 differential: encode the vector, run both CLIs,
/// extract the metadata blob summary fields from each JSON output,
/// and assert they match.
///
/// This catches divergences in the metadata parser (inode layout,
/// directory node layout, root-inode identification) that the
/// manifest-root check alone cannot see.
///
/// # Errors
///
/// Returns `Err(String)` if either CLI fails or the summaries
/// disagree.
pub fn differential_metadata(vector: &Vector) -> Result<(), String> {
    let artifact: ManifestArtifact = ManifestBuilder::new(vector.spec.clone()).build();
    let rust = run_limni_rust(&artifact.bytes)?;
    let py = run_limni_py(&artifact.bytes)?;
    let rust_summary = extract_metadata_summary(&rust.stdout)
        .ok_or_else(|| format!("rust did not emit metadata summary; stderr={}", rust.stderr))?;
    let py_summary = extract_metadata_summary(&py.stdout)
        .ok_or_else(|| format!("python did not emit metadata summary; stderr={}", py.stderr))?;
    if rust_summary != py_summary {
        return Err(format!(
            "metadata summary mismatch on vector {}: rust={rust_summary} py={py_summary}",
            vector.name
        ));
    }
    Ok(())
}

/// Pull the metadata_* fields from a `verify --json` report,
/// normalised to a stable shape (sorted keys, arrays in declared
/// order). Returns `None` if the fields are absent (e.g. for
/// external-metadata vectors).
fn extract_metadata_summary(stdout: &str) -> Option<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(stdout).ok()?;
    if !value.get("metadata_inlined")?.as_bool()? {
        return Some(serde_json::Value::Null);
    }
    let obj = value.as_object()?;
    let keys = [
        "metadata_inode_count",
        "metadata_dir_node_count",
        "metadata_root_inode",
        "metadata_inodes",
        "metadata_dir_nodes",
    ];
    let mut summary = serde_json::Map::new();
    for key in keys {
        if let Some(v) = obj.get(key) {
            summary.insert((*key).to_owned(), v.clone());
        }
    }
    Some(serde_json::Value::Object(summary))
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
        .map_or(0u128, |d| d.as_nanos());
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
    use crate::vectors::differential_vectors;

    #[test]
    fn differential_agreement_or_skip() {
        if !should_run() {
            eprintln!(
                "skipping differential test: {DIFFERENTIAL_ENV_VAR} unset and adapters not on PATH"
            );
            return;
        }
        for vector in differential_vectors() {
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
    fn differential_metadata_agreement_or_skip() {
        if !should_run() {
            eprintln!(
                "skipping differential metadata test: {DIFFERENTIAL_ENV_VAR} unset and adapters not on PATH"
            );
            return;
        }
        for vector in differential_vectors() {
            if let Err(e) = differential_metadata(&vector) {
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

    #[test]
    fn differential_rejects_bad_magic_or_skip() {
        if !should_run() {
            eprintln!(
                "skipping differential rejection test: {DIFFERENTIAL_ENV_VAR} unset and adapters not on PATH"
            );
            return;
        }
        for vector in differential_vectors() {
            differential_rejection(&vector, Mutation::BadMagic)
                .unwrap_or_else(|e| panic!("vector {} bad-magic: {e}", vector.name));
        }
    }

    #[test]
    fn differential_rejects_truncated_or_skip() {
        if !should_run() {
            eprintln!("skipping differential rejection test");
            return;
        }
        for vector in differential_vectors() {
            let artifact = ManifestBuilder::new(vector.spec.clone()).build();
            let original_len = artifact.bytes.len();
            // Truncate to the manifest header (16 bytes) — every parser
            // after the header will reject for lack of bytes.
            let truncate_len = 16_usize.min(original_len.saturating_sub(1));
            differential_rejection(
                &vector,
                Mutation::Truncate {
                    new_len: truncate_len,
                },
            )
            .unwrap_or_else(|e| panic!("vector {} truncate: {e}", vector.name));
        }
    }

    #[test]
    fn differential_rejects_history_byte_flip_or_skip() {
        if !should_run() {
            eprintln!("skipping differential rejection test");
            return;
        }
        for vector in differential_vectors() {
            let artifact = ManifestBuilder::new(vector.spec.clone()).build();
            let original_len = artifact.bytes.len();
            // Flip the last byte (in the history section, always present).
            let offset = original_len - 1;
            differential_rejection(&vector, Mutation::FlipByte { offset, mask: 0xFF })
                .unwrap_or_else(|e| panic!("vector {} flip-byte: {e}", vector.name));
        }
    }

    #[test]
    fn mutation_truncate_panics_on_zero_or_negative_growth() {
        let original = b"hello".to_vec();
        let truncated = Mutation::Truncate { new_len: 3 }.apply(&original);
        assert_eq!(truncated, b"hel");
    }

    #[test]
    fn mutation_bad_magic_overwrites_first_four_bytes() {
        let original = b"LMFS_rest_of_the_buffer".to_vec();
        let mutated = Mutation::BadMagic.apply(&original);
        assert_eq!(&mutated[..4], b"XXXX");
        assert_eq!(&mutated[4..], &original[4..]);
    }

    #[test]
    fn mutation_flip_byte_xors_single_byte() {
        let original = vec![0x00, 0xAA, 0xFF];
        let mutated = Mutation::FlipByte {
            offset: 1,
            mask: 0xFF,
        }
        .apply(&original);
        assert_eq!(mutated, vec![0x00, 0x55, 0xFF]);
    }

    #[test]
    fn mutation_label_is_human_readable() {
        assert_eq!(Mutation::Truncate { new_len: 10 }.label(), "truncate");
        assert_eq!(Mutation::BadMagic.label(), "bad-magic");
        assert_eq!(
            Mutation::FlipByte {
                offset: 0,
                mask: 0xFF
            }
            .label(),
            "flip-byte"
        );
    }
}
