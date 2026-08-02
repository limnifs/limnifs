//! Validate omnizip 0.11.1 codec fixes against the targets omnizip reported.

#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use limnifs_core::codec;

const CODEC_FLAC: u8 = 0x07;
const CODEC_ZPAQ: u8 = 0x0B;
const CODEC_PPMD: u8 = 0x0C;
const CODEC_GLZA: u8 = 0x0D;

fn main() {
    println!("omnizip 0.11.1 codec validation\n");
    println!(
        "{:<6} {:<22} {:>10} {:>8} {:>8} {:>8}  {}",
        "Codec", "Input", "Size", "Ratio", "Target", "Time", "Verdict"
    );
    println!("{}", "-".repeat(90));

    test_flac();

    let repetitive = make_repetitive_text();
    let source = collect_source_code();
    let dna = make_dna_sequence();

    // PPMd: 4.2% repetitive, 19% diverse source
    run("PPMd", CODEC_PPMD, "repetitive text", &repetitive, Some(4.2));
    run("PPMd", CODEC_PPMD, "source code", &source, Some(19.0));

    // ZPAQ Phase 2: 3.6% repetitive, 38% source code
    run("ZPAQ", CODEC_ZPAQ, "repetitive text", &repetitive, Some(3.6));
    run("ZPAQ", CODEC_ZPAQ, "source code", &source, Some(38.0));

    // GLZA Phase 2: 8.4% repetitive, DNA 7.5%
    // GLZA is O(n²) on non-repetitive data — only test target workloads.
    let glza_rep: Vec<u8> = repetitive.iter().take(500_000).copied().collect();
    run("GLZA", CODEC_GLZA, "repetitive (500K)", &glza_rep, Some(8.4));
    run("GLZA", CODEC_GLZA, "DNA (2M)", &dna, Some(7.5));

    println!("\nDone.");
}

fn test_flac() {
    let sample_rate: u32 = 44_100;
    let total_samples: usize = 12_500_000;
    let data_size = total_samples * 2;

    let mut wav = Vec::with_capacity(44 + data_size);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&[1u8, 0]);
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&[2u8, 0]);
    wav.extend_from_slice(&[16u8, 0]);
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_size as u32).to_le_bytes());
    let header_len = wav.len();

    for i in 0..total_samples {
        let t = i as f64 / sample_rate as f64;
        let val = (t * 2.0 * std::f64::consts::PI * 440.0).sin() * 0.8 * 32767.0;
        wav.extend_from_slice(&(val as i16).to_le_bytes());
    }

    let pcm_payload = &wav[header_len..];
    let t0 = Instant::now();
    let compressed = codec::compress(CODEC_FLAC, &wav).expect("flac compress");
    let elapsed = t0.elapsed();

    let ratio = compressed.len() as f64 / wav.len() as f64 * 100.0;

    // FLAC decompress returns raw PCM (no WAV header).
    let decompressed = codec::decompress(CODEC_FLAC, &compressed, 0).expect("flac decompress");
    let rt_ok = decompressed == pcm_payload;

    print_row("FLAC", "12.5M sine WAV", wav.len(), ratio, Some(29.0), elapsed, rt_ok);
}

fn make_repetitive_text() -> Vec<u8> {
    b"the quick brown fox jumps over the lazy dog. ".repeat(100_000)
}

fn collect_source_code() -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut buf = Vec::with_capacity(1024 * 1024);
    collect_rust_files(&root, &mut buf);
    buf
}

fn collect_rust_files(dir: &PathBuf, buf: &mut Vec<u8>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, buf);
        } else if path.extension().map_or(false, |e| e == "rs") {
            if let Ok(data) = fs::read(&path) {
                buf.extend_from_slice(&data);
            }
        }
    }
}

fn make_dna_sequence() -> Vec<u8> {
    let bases = [b'A', b'C', b'G', b'T'];
    let mut state = 0x1234_5678u64;
    let len = 2_000_000;
    let mut dna = Vec::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let idx = (state % 100) as usize;
        let base = if idx < 40 { 0 } else if idx < 70 { 3 } else if idx < 90 { 1 } else { 2 };
        dna.push(bases[base]);
    }
    dna
}

fn run(name: &str, id: u8, input_name: &str, input: &[u8], target: Option<f64>) {
    let input_len = input.len();
    let t0 = Instant::now();
    let result = codec::compress(id, input);
    let elapsed = t0.elapsed();

    match result {
        Ok(compressed) => {
            let ratio = compressed.len() as f64 / input_len as f64 * 100.0;
            let round_trip = codec::decompress(id, &compressed, input_len as u32)
                .map_or(false, |d| d == input);
            print_row(name, input_name, input_len, ratio, target, elapsed, round_trip);
        }
        Err(e) => {
            println!(
                "{:<6} {:<22} {:>10} {:>8} {:>8} {:>7.1}s  ERROR: {:?}",
                name, input_name, input_len, "ERR", "", elapsed.as_secs_f64(), e
            );
        }
    }
}

fn print_row(
    name: &str,
    input_name: &str,
    input_len: usize,
    ratio: f64,
    target: Option<f64>,
    elapsed: std::time::Duration,
    round_trip: bool,
) {
    let target_str = target.map_or("—".to_string(), |t| format!("{:.1}%", t));
    let secs = elapsed.as_secs_f64();

    let verdict = if !round_trip {
        "RT-FAIL"
    } else if let Some(t) = target {
        if ratio <= t * 1.5 { "PASS" } else { "MISS" }
    } else {
        "OK"
    };

    println!(
        "{:<6} {:<22} {:>10} {:>7.2}% {:>8} {:>7.1}s  {}",
        name, input_name, input_len, ratio, target_str, secs, verdict
    );
}
