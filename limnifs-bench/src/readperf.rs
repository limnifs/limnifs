//! Read-path performance canaries (TODO.sota-fs/06).
//!
//! Fixed synthetic image, fixed workload, hard gates. Regression in
//! the read path (windowed decode, cache policy, seekable container)
//! shows up here as a red CI job instead of a user-reported 48 GiB of
//! wasted decompression (limnifs#192).
//!
//! Workload:
//! - 4 files × 8 MiB pseudo-random content (multi-drop images —
//!   FastCDC chunks each file; dedup keeps drop count honest).
//! - (a) sequential extract throughput via `ImageReader` +
//!   `read_to_end`.
//! - (b) warm 8 KiB random-window throughput — the tebako #192
//!   access pattern, served by the SIEVE drop cache + seekable
//!   containers.
//!
//! Gates (exit nonzero): windowed ≥ 200 MB/s, extract ≥ 100 MB/s.
//! Total runtime budget: < 60 s.

use std::io::Read;
use std::path::PathBuf;
use std::time::Instant;

use limnifs_core::read::{ImageReader, ReadConfig};
use limnifs_write::{write_directory_with_config, WriteConfig};

/// Gate: warm windowed random-read effective throughput floor.
pub const WINDOWED_GATE_MBPS: f64 = 200.0;
/// Gate: sequential extract throughput floor.
pub const EXTRACT_GATE_MBPS: f64 = 100.0;

/// Per-file synthetic content size.
const FILE_BYTES: usize = 8 * 1024 * 1024;
/// Number of files.
const FILE_COUNT: usize = 4;
/// Window size for the random-read phase — matches the tebako #192
/// FUSE read size.
const WINDOW: usize = 8 * 1024;
/// Windows sampled in the warm phase.
const WARM_WINDOWS: usize = 4096;

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

struct BuiltImage {
    dir: PathBuf,
    total_bytes: u64,
}

fn build_image(root: &PathBuf) -> BuiltImage {
    let src = root.join("readperf-src");
    std::fs::create_dir_all(&src).expect("mkdir src");
    for i in 0..FILE_COUNT {
        let data = xorshift(FILE_BYTES, 0x1000 + u64::try_from(i).unwrap_or(0) * 7919);
        std::fs::write(src.join(format!("file{i}.bin")), &data).expect("write fixture");
    }

    let mut config = WriteConfig::default_v0_1();
    // LZ4 keeps the fixture build inside the CI time budget without
    // changing the read-path characteristics being measured.
    config.defaults.text_codec = "lz4".into();
    config.defaults.binary_codec = "lz4".into();
    let art = write_directory_with_config(&src, &config).expect("pack fixture");

    let dir = root.join("readperf-img");
    std::fs::create_dir_all(&dir).expect("mkdir img");
    std::fs::write(dir.join("image.lim"), &art.bytes).expect("manifest");
    for slab in &art.slabs {
        let name =
            limnifs_core::locator::local_sidecar_name(&slab.locator).expect("flat slab locator");
        std::fs::write(dir.join(name), &slab.bytes).expect("slab");
    }
    if let Some(side) = &art.metadata_sidecar {
        let name =
            limnifs_core::locator::local_sidecar_name(&side.locator).expect("flat sidecar name");
        std::fs::write(dir.join(name), &side.bytes).expect("metadata sidecar");
    }
    BuiltImage {
        dir,
        total_bytes: (FILE_BYTES * FILE_COUNT) as u64,
    }
}

/// Run the canaries; returns `(extract_mbps, windowed_mbps)`.
///
/// # Panics
///
/// Panics on fixture or reader failure — the canary must fail loudly,
/// not return a zero throughput that quietly passes gates.
pub fn run(root: &PathBuf) -> (f64, f64) {
    let image = build_image(root);
    let manifest = image.dir.join("image.lim");

    // (a) Sequential extract.
    let reader = ImageReader::open(&manifest, ReadConfig::default()).expect("open");
    let mut sink = Vec::with_capacity(FILE_BYTES);
    let start = Instant::now();
    let mut extracted: u64 = 0;
    for i in 0..FILE_COUNT {
        let mut file = reader
            .file(&format!("/file{i}.bin"))
            .expect("file resolves");
        sink.clear();
        file.read_to_end(&mut sink).expect("extract");
        extracted += sink.len() as u64;
    }
    let extract_secs = start.elapsed().as_secs_f64();
    let extract_mbps = extracted as f64 / (1024.0 * 1024.0) / extract_secs.max(f64::EPSILON);

    // (b) Warm 8 KiB random windows (fresh reader → warm-up pass
    // populates the cache, then the measured pass is served warm).
    let reader = ImageReader::open(&manifest, ReadConfig::default()).expect("open");
    let mut files: Vec<_> = (0..FILE_COUNT)
        .map(|i| reader.file(&format!("/file{i}.bin")).expect("file"))
        .collect();
    let mut window = vec![0u8; WINDOW];
    let mut state = 0xFEED_FACE_CAFE_BABEu64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    // Warm-up: one full sequential pass per file.
    let mut warm_sink = Vec::with_capacity(FILE_BYTES);
    for file in &mut files {
        warm_sink.clear();
        file.read_to_end(&mut warm_sink).expect("warm-up read");
    }
    // Measured: random (file, offset) windows.
    let start = Instant::now();
    let mut moved: usize = 0;
    for _ in 0..WARM_WINDOWS {
        let f = (next() % FILE_COUNT as u64) as usize;
        let off = next() % (FILE_BYTES as u64 - WINDOW as u64);
        let n = files[f].read_at(off, &mut window).expect("windowed read");
        moved += n;
    }
    let windowed_secs = start.elapsed().as_secs_f64();
    let windowed_mbps = moved as f64 / (1024.0 * 1024.0) / windowed_secs.max(f64::EPSILON);

    println!(
        "readperf: extract {extract_mbps:.0} MB/s (gate ≥ {EXTRACT_GATE_MBPS:.0}), \
         windowed-8KiB {windowed_mbps:.0} MB/s (gate ≥ {WINDOWED_GATE_MBPS:.0})"
    );
    let _ = std::fs::remove_dir_all(image.dir.join("").join("readperf-src"));
    (extract_mbps, windowed_mbps)
}
