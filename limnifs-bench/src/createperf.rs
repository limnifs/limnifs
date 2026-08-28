//! Create-path performance canary with hard gates.
//!
//! The read path has readperf (windowed ≥ 200 MB/s, extract ≥
//! 100 MB/s); until now nothing gated the create path, whose shipped
//! wins (create-path mmap borrow, parallel Phase-1 hashing, parallel
//! dictionary re-compression) would regress silently — the v0.3.3
//! materialization bug (a full memcpy per mapped file) is the exact
//! class this canary exists to catch.
//!
//! Workload: fixed synthetic tree — 6 compressible text-like files +
//! 2 incompressible random files, 8 MiB each (64 MiB total), LZ4
//! config, default dictionary training. Measures
//! `write_directory_with_config` wall-clock throughput.
//!
//! Gate (exit nonzero below): create ≥ 50 MB/s. Conservative on
//! purpose — it must catch order-of-magnitude regressions, not
//! runner-to-runner jitter. Total runtime budget: < 30 s.

use std::path::PathBuf;
use std::time::Instant;

use limnifs_write::{write_directory_with_config, WriteConfig};

/// Gate: create (pack) throughput floor.
pub const CREATE_GATE_MBPS: f64 = 50.0;

/// Per-file synthetic content size.
const FILE_BYTES: usize = 8 * 1024 * 1024;
/// Compressible (text-like) files.
const TEXT_FILES: usize = 6;
/// Incompressible (random) files.
const RANDOM_FILES: usize = 2;

fn xorshift(bytes: usize, seed: u64) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(bytes);
    while out.len() < bytes {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(bytes);
    out
}

/// Text-like fixture: compressible, deterministic, varied enough that
/// FastCDC cuts several boundaries per file.
fn text_like(bytes: usize, seed: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes);
    let mut line = seed;
    while out.len() < bytes {
        line = line
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let s = format!("limnifs createperf line {line:020} seed {seed:04x}\n");
        out.extend_from_slice(s.as_bytes());
    }
    out.truncate(bytes);
    out
}

/// Run the canary; returns create throughput in MB/s.
///
/// # Panics
///
/// Panics on fixture or packer failure — the canary must fail loudly,
/// not return a zero throughput that quietly passes the gate.
pub fn run(root: &PathBuf) -> f64 {
    let src = root.join("createperf-src");
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(&src).expect("mkdir src");
    for i in 0..TEXT_FILES {
        let data = text_like(FILE_BYTES, 0xA000 + u64::try_from(i).unwrap_or(0) * 131);
        std::fs::write(src.join(format!("text{i}.txt")), &data).expect("write fixture");
    }
    for i in 0..RANDOM_FILES {
        let data = xorshift(FILE_BYTES, 0xB000 + u64::try_from(i).unwrap_or(0) * 911);
        std::fs::write(src.join(format!("rand{i}.bin")), &data).expect("write fixture");
    }
    let total_bytes = (FILE_BYTES * (TEXT_FILES + RANDOM_FILES)) as u64;

    let mut config = WriteConfig::default_v0_1();
    // LZ4 matches readperf's fixture policy: keeps the canary inside
    // the CI time budget without changing the create-path stages
    // being measured (chunk → hash → tournament → dict → assemble).
    config.defaults.text_codec = "lz4".into();
    config.defaults.binary_codec = "lz4".into();

    // One unmeasured warm-up pack to populate the page cache; the
    // measured passes read warm, matching how readperf measures.
    let _ = write_directory_with_config(&src, &config).expect("warm-up pack");

    // Three measured packs, median judged: a single reading from a
    // shared host carries no confidence interval — this week produced
    // three separate forensic sessions over "regression or co-tenant?"
    // that a spread line would have settled at a glance.
    let mut mbps_samples = [0.0f64; 3];
    for slot in &mut mbps_samples {
        let start = Instant::now();
        let _ = write_directory_with_config(&src, &config).expect("pack fixture");
        let secs = start.elapsed().as_secs_f64();
        *slot = total_bytes as f64 / (1024.0 * 1024.0) / secs.max(f64::EPSILON);
    }
    mbps_samples.sort_by(|a, b| a.total_cmp(b));
    let (min, median, max) = (mbps_samples[0], mbps_samples[1], mbps_samples[2]);
    let spread = if median > 0.0 {
        (max - min) / median
    } else {
        0.0
    };

    let art = write_directory_with_config(&src, &config).expect("pack fixture");
    let slab_bytes: u64 = art.slabs.iter().map(|s| s.bytes.len() as u64).sum();
    let noise_note = if spread > 0.25 {
        " [noisy host — CI canary is the arbiter]"
    } else {
        ""
    };
    println!(
        "createperf: pack {median:.0} MB/s ({min:.0}…{max:.0}, spread {:.0}%) \
         (gate ≥ {CREATE_GATE_MBPS:.0}){noise_note}, 64 MiB tree → {slab_bytes} slab bytes",
        spread * 100.0
    );
    let _ = std::fs::remove_dir_all(&src);
    median
}
