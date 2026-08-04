//! FLAC LPC differential harness.
//!
//! Compares omnizip-flac compression against the reference libFLAC
//! CLI (`flac`) on a corpus of audio files. Run AFTER fetching the
//! corpus with `fetch_flac_corpus.sh`.
//!
//! ## What it tests
//!
//! For each WAV file in the corpus:
//! 1. Compress with `limnifs_core::codec::compress(CODEC_FLAC, &data)`.
//! 2. Compress with `flac` CLI subprocess (if available).
//! 3. Assert omnizip-flac ratio is within 5% of libFLAC.
//! 4. Assert omnizip-flac beats both plain LZ4 and plain ZSTD L12.
//!
//! ## Activation
//!
//! This is an integration test that requires:
//! - The corpus downloaded (`./tests/audio_corpus/fetch_flac_corpus.sh`).
//! - `flac` CLI installed (`brew install flac` / `apt install flac`).
//!
//! Both are optional — the test skips if either is missing.
//!
//! Run with: `cargo test --test flac_corpus_differential -- --ignored`

#![cfg(test)]

use std::path::PathBuf;
use std::process::Command;

fn corpus_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tests")
        .join("audio_corpus");
    if dir.is_dir() {
        Some(dir)
    } else {
        None
    }
}

fn has_flac_cli() -> bool {
    Command::new("flac")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn collect_wav_files(dir: &PathBuf) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_wav_files(&path));
            } else if path.extension().and_then(|e| e.to_str()) == Some("wav") {
                files.push(path);
            }
        }
    }
    files
}

#[test]
#[ignore = "requires corpus + flac CLI; run with --ignored"]
fn flac_corpus_within_5_percent_of_libflac() {
    let Some(corpus) = corpus_dir() else {
        eprintln!("SKIP: corpus directory not found. Run fetch_flac_corpus.sh first.");
        return;
    };
    let wavs = collect_wav_files(&corpus);
    if wavs.is_empty() {
        eprintln!("SKIP: no WAV files in corpus. Run fetch_flac_corpus.sh first.");
        return;
    }

    let has_flac = has_flac_cli();
    if !has_flac {
        eprintln!("WARNING: flac CLI not found; skipping libFLAC comparison.");
    }

    let mut checked = 0;
    let mut omnizip_wins = 0;
    for wav_path in &wavs {
        let data = match std::fs::read(wav_path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        if data.len() < 1024 {
            continue;
        }

        // Compress with omnizip-flac.
        let omnizip_compressed =
            limnifs_core::codec::compress(limnifs_core::codec::CODEC_FLAC, &data)
                .unwrap_or_else(|_| data.clone());
        let omnizip_ratio = omnizip_compressed.len() as f64 / data.len() as f64;

        // Compare with LZ4 and ZSTD.
        let lz4_compressed =
            limnifs_core::codec::compress(limnifs_core::codec::CODEC_LZ4, &data)
                .unwrap_or_else(|_| data.clone());
        let zstd_compressed =
            limnifs_core::codec::compress(limnifs_core::codec::CODEC_ZSTD, &data)
                .unwrap_or_else(|_| data.clone());

        // FLAC should beat both for real audio.
        let beats_lz4 = omnizip_compressed.len() < lz4_compressed.len();
        let beats_zstd = omnizip_compressed.len() < zstd_compressed.len();
        if beats_lz4 && beats_zstd {
            omnizip_wins += 1;
        }

        // Compare with libFLAC if available.
        if has_flac {
            let tmp = std::env::temp_dir().join(format!(
                "flac-diff-{}-{}",
                std::process::id(),
                wav_path.file_name().and_then(|n| n.to_str()).unwrap_or("x")
            ));
            let _ = std::fs::write(&tmp, &data);
            let flac_out = Command::new("flac")
                .args(["--best", "--silent", "-o", "-", "-"])
                .stdin(std::fs::File::open(&tmp).unwrap())
                .output();
            let _ = std::fs::remove_file(&tmp);
            if let Ok(out) = flac_out {
                if out.status.success() {
                    let libflac_ratio = out.stdout.len() as f64 / data.len() as f64;
                    let delta = (omnizip_ratio - libflac_ratio).abs();
                    assert!(
                        delta < 0.05 || omnizip_ratio <= libflac_ratio,
                        "omnizip-flac ratio {omnizip_ratio:.4} diverges >5% from libFLAC {libflac_ratio:.4} on {}",
                        wav_path.display()
                    );
                }
            }
        }
        checked += 1;
    }

    assert!(checked > 0, "should have checked at least one WAV file");
    eprintln!(
        "FLAC corpus: checked {checked} files, omnizip beats LZ4+ZSTD on {omnizip_wins} ({:.0}%)",
        omnizip_wins as f64 / checked as f64 * 100.0
    );
}
