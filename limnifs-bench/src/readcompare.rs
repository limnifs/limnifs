//! Monolithic vs seekable A/B benchmark (TODO.sota-fs/10).
//!
//! The same 19.5 MiB fixture (the limnifs#192 scenario size) packed
//! twice with identical config except `defaults.seekable_drops`:
//! monolithic = one whole-file codec stream ("v1 behavior"),
//! seekable = 256 KiB frames + footer ("v2 behavior"). Metrics:
//! first-window latency, cold windowed MB/s, warm windowed MB/s,
//! sequential extract MB/s, image bytes, and decoded-frame counts.
//!
//! Informational — hard gates live in `readperf`.

use std::io::Read;
use std::path::PathBuf;
use std::time::Instant;

use limnifs_core::read::{ImageReader, ReadConfig};
use limnifs_core::seekable;
use limnifs_write::{write_directory_with_config, WriteConfig};

const FILE_BYTES: usize = 19 * 1024 * 1024 + 512 * 1024; // 19.5 MiB
const WINDOW: usize = 8 * 1024;
const COLD_WINDOWS: usize = 32;
const WARM_WINDOWS: usize = 2048;

fn compressible(bytes: usize) -> Vec<u8> {
    let mut state = 0x0123_4567_89AB_CDEFu64;
    let mut out = Vec::with_capacity(bytes);
    while out.len() < bytes {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(&[state.to_le_bytes()[7]; 8]);
    }
    out.truncate(bytes);
    out
}

struct Packed {
    manifest: PathBuf,
    image_bytes: u64,
}

fn pack(root: &PathBuf, seekable_drops: bool, payload: &[u8]) -> Packed {
    let tag = if seekable_drops {
        "seekable"
    } else {
        "monolithic"
    };
    let src = root.join(format!("{tag}-src"));
    std::fs::create_dir_all(&src).expect("mkdir src");
    std::fs::write(src.join("big.bin"), payload).expect("write fixture");

    let mut config = WriteConfig::default_v0_1();
    config.skip_chunking = true; // whole-file drop either way
    config.defaults.text_codec = "lz4".into();
    config.defaults.binary_codec = "lz4".into();
    config.defaults.seekable_drops = seekable_drops;
    let art = write_directory_with_config(&src, &config).expect("pack");

    let dir = root.join(format!("{tag}-img"));
    std::fs::create_dir_all(&dir).expect("mkdir img");
    std::fs::write(dir.join("image.lim"), &art.bytes).expect("manifest");
    let mut image_bytes = art.bytes.len() as u64;
    for slab in &art.slabs {
        image_bytes += slab.bytes.len() as u64;
        let name =
            limnifs_core::locator::local_sidecar_name(&slab.locator).expect("flat slab locator");
        std::fs::write(dir.join(name), &slab.bytes).expect("slab");
    }
    if let Some(side) = &art.metadata_sidecar {
        image_bytes += side.bytes.len() as u64;
        let name =
            limnifs_core::locator::local_sidecar_name(&side.locator).expect("flat sidecar name");
        std::fs::write(dir.join(name), &side.bytes).expect("metadata");
    }
    let _ = std::fs::remove_dir_all(&src);
    Packed {
        manifest: dir.join("image.lim"),
        image_bytes,
    }
}

/// Cold-cache reader config: every drop and frame bypasses, so each
/// window pays its real decode cost.
fn cold_config() -> ReadConfig {
    ReadConfig {
        cache_entries: 1,
        cache_bytes: 1,
        parallel_decode: false,
        frame_cache_bytes: 1,
    }
}

struct Metrics {
    first_window_us: f64,
    cold_mbps: f64,
    cold_frames: u64,
    warm_mbps: f64,
    extract_mbps: f64,
    image_bytes: u64,
}

fn measure(packed: &Packed) -> Metrics {
    // (1) First-window latency: the decode cost of the very first
    // 8 KiB pread on a cold reader.
    let reader = ImageReader::open(&packed.manifest, cold_config()).expect("open");
    let file = reader.file("/big.bin").expect("file");
    let mut window = vec![0u8; WINDOW];
    let before = seekable::frames_decoded();
    let start = Instant::now();
    let n = file
        .read_at(FILE_BYTES as u64 / 2, &mut window)
        .expect("first read");
    let first_window_us = start.elapsed().as_secs_f64() * 1e6;
    assert_eq!(n, WINDOW);
    let first_frames = seekable::frames_decoded() - before;

    // (2) Cold windowed throughput + frame count.
    let reader = ImageReader::open(&packed.manifest, cold_config()).expect("open");
    let file = reader.file("/big.bin").expect("file");
    let mut state = 0xDEAD_BEEF_CAFE_F00Du64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let before = seekable::frames_decoded();
    let start = Instant::now();
    let mut moved = 0usize;
    for _ in 0..COLD_WINDOWS {
        let off = next() % (FILE_BYTES as u64 - WINDOW as u64);
        let n = file.read_at(off, &mut window).expect("cold read");
        moved += n;
    }
    let cold_secs = start.elapsed().as_secs_f64();
    let cold_frames = seekable::frames_decoded() - before + first_frames;
    let cold_mbps = moved as f64 / (1024.0 * 1024.0) / cold_secs.max(f64::EPSILON);

    // (3) Warm windowed throughput: default caches after a
    // sequential warm-up pass.
    let reader = ImageReader::open(&packed.manifest, ReadConfig::default()).expect("open");
    let mut file = reader.file("/big.bin").expect("file");
    let mut sink = Vec::with_capacity(FILE_BYTES);
    file.read_to_end(&mut sink).expect("warm-up");
    drop(file);
    let file = reader.file("/big.bin").expect("file");
    let start = Instant::now();
    let mut moved = 0usize;
    for _ in 0..WARM_WINDOWS {
        let off = next() % (FILE_BYTES as u64 - WINDOW as u64);
        let n = file.read_at(off, &mut window).expect("warm read");
        moved += n;
    }
    let warm_secs = start.elapsed().as_secs_f64();
    let warm_mbps = moved as f64 / (1024.0 * 1024.0) / warm_secs.max(f64::EPSILON);

    // (4) Sequential extract on a cold reader.
    let reader = ImageReader::open(&packed.manifest, ReadConfig::default()).expect("open");
    let mut file = reader.file("/big.bin").expect("file");
    let mut out = Vec::with_capacity(FILE_BYTES);
    let start = Instant::now();
    file.read_to_end(&mut out).expect("extract");
    let extract_secs = start.elapsed().as_secs_f64();
    assert_eq!(out.len(), FILE_BYTES);
    let extract_mbps = out.len() as f64 / (1024.0 * 1024.0) / extract_secs.max(f64::EPSILON);

    Metrics {
        first_window_us,
        cold_mbps,
        cold_frames,
        warm_mbps,
        extract_mbps,
        image_bytes: packed.image_bytes,
    }
}

/// Run the A/B comparison; prints the table.
///
/// # Panics
///
/// Panics on fixture or reader failure — a benchmark that silently
/// returns zeroes is worse than one that crashes.
pub fn run(root: &PathBuf) {
    let payload = compressible(FILE_BYTES);
    let mono = measure(&pack(root, false, &payload));
    let seek = measure(&pack(root, true, &payload));

    println!("\nreadcompare — monolithic vs seekable (19.5 MiB drop, 8 KiB windows)\n");
    println!(
        "{:<26}{:>14}{:>14}{:>12}",
        "metric", "monolithic", "seekable", "delta"
    );
    println!("{}", "-".repeat(66));
    let row = |name: &str, m: f64, s: f64, unit: &str| {
        let ratio = if m > 0.0 { s / m } else { f64::INFINITY };
        println!(
            "{:<26}{:>11.1}{:>3}{:>11.1}{:>3}{:>9.2}x",
            name, m, unit, s, unit, ratio
        );
    };
    row(
        "first window",
        mono.first_window_us,
        seek.first_window_us,
        "us",
    );
    row("cold windowed", mono.cold_mbps, seek.cold_mbps, "MB");
    row("warm windowed", mono.warm_mbps, seek.warm_mbps, "MB");
    row(
        "sequential extract",
        mono.extract_mbps,
        seek.extract_mbps,
        "MB",
    );
    println!(
        "{:<26}{:>14}{:>14}{:>12}",
        "container frames decoded",
        "-",
        seek.cold_frames,
        format!(
            "{:.2}/window",
            seek.cold_frames as f64 / COLD_WINDOWS as f64
        )
    );
    println!(
        "{:<26}{:>11.2}{:>3}{:>11.2}{:>3}{:>9.2}x",
        "image size",
        mono.image_bytes as f64 / (1024.0 * 1024.0),
        "MB",
        seek.image_bytes as f64 / (1024.0 * 1024.0),
        "MB",
        seek.image_bytes as f64 / mono.image_bytes as f64
    );
    println!(
        "\ncold work per 8 KiB window: monolithic decodes the whole {:.1} MiB drop; \
         seekable decodes {:.2} x 256 KiB frames.",
        FILE_BYTES as f64 / (1024.0 * 1024.0),
        seek.cold_frames as f64 / COLD_WINDOWS as f64
    );
}
