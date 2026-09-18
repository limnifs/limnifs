//! Warm-start window probe (TODO.features/35).
//!
//! Settles whether seeding chunk N's encoder with chunk N-1's tail
//! can EVER be net-positive at LimniFS chunk sizes, before any
//! upstream omnizip-zstd API work:
//!
//! - COST is exact: a self-contained warm frame must contain the
//!   window (zstd frames reference only their own content), so the
//!   floor is `n_chunks x W` bytes of raw-block prefix.
//! - GAIN is an upper bound: greedy longest-match coverage of each
//!   chunk against the previous chunk's W-tail (8-byte anchors) —
//!   a warm encoder cannot save more than these literals.
//!
//! net = gain - cost. Run: `cargo run -p limnifs-bench --example
//! warm_window_probe -- <file>...` (or `random:BYTES` for the
//! incompressible control).

use std::collections::HashMap;

use limnifs_core::codec::{compress, CODEC_ZSTD};
use limnifs_write::chunker::{Chunker as _, ParallelFastCDC};

const MIN_MATCH: usize = 8;
const WINDOWS: [usize; 5] = [4 * 1024, 8 * 1024, 16 * 1024, 32 * 1024, 64 * 1024];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: warm_window_probe <file|random:BYTES>...");
        std::process::exit(2);
    }
    for arg in &args {
        let data = if let Some(size) = arg.strip_prefix("random:") {
            let size: usize = size.parse().expect("random:BYTES");
            let mut v = Vec::with_capacity(size);
            let mut state = 0x5EED_5EED_5EED_5EEDu64;
            while v.len() < size {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                v.push((state >> 56) as u8);
            }
            v
        } else {
            std::fs::read(arg).unwrap_or_else(|e| panic!("read {arg}: {e}"))
        };
        probe(arg, &data);
    }
}

fn probe(label: &str, data: &[u8]) {
    let chunker = ParallelFastCDC::new(65_536, 262_144, 1_048_576).expect("default chunking");
    let chunks = chunker.chunk_slice(data);
    let n = chunks.len();
    let baseline: usize = chunks
        .iter()
        .map(|c| compress(CODEC_ZSTD, c).expect("zstd").len())
        .sum();
    println!(
        "\n{label}: {} bytes, {n} chunks (avg {} KiB), zstd-Fastest baseline {baseline} bytes ({:.2}%)",
        data.len(),
        data.len() / n.max(1) / 1024,
        100.0 * baseline as f64 / data.len() as f64
    );
    println!("  window    cost(+W/chunk)   gain(upper bnd)   net vs baseline");
    for w in WINDOWS {
        let cost = n * w;
        let gain: usize = chunks
            .windows(2)
            .map(|pair| match_coverage(pair[1], &pair[0][pair[0].len().saturating_sub(w)..]))
            .sum();
        let net = gain as i64 - cost as i64;
        println!(
            "  {:>4} KiB  {:>10} B  {:>9} B ({:>5.2}%)  {}",
            w / 1024,
            cost,
            gain,
            100.0 * gain as f64 / baseline as f64,
            if net > 0 {
                format!("POSITIVE {:+.2}%", 100.0 * net as f64 / baseline as f64)
            } else {
                format!("{:+.2}%", 100.0 * net as f64 / baseline as f64)
            }
        );
    }
}

/// Greedy longest-match coverage of `chunk` against `window`:
/// bytes of `chunk` inside a >=8-byte match into `window`. An
/// upper bound on what a warm encoder could turn into references.
fn match_coverage(chunk: &[u8], window: &[u8]) -> usize {
    if window.len() < MIN_MATCH || chunk.len() < MIN_MATCH {
        return 0;
    }
    // Anchor table: every MIN_MATCH-length substring of the window.
    let mut table: HashMap<&[u8], Vec<usize>> = HashMap::new();
    for i in 0..=window.len() - MIN_MATCH {
        table.entry(&window[i..i + MIN_MATCH]).or_default().push(i);
    }
    let mut covered = 0usize;
    let mut i = 0usize;
    while i + MIN_MATCH <= chunk.len() {
        if let Some(positions) = table.get(&chunk[i..i + MIN_MATCH]) {
            let mut best = 0usize;
            for &j in positions {
                let mut len = 0usize;
                while i + len < chunk.len()
                    && j + len < window.len()
                    && chunk[i + len] == window[j + len]
                {
                    len += 1;
                }
                best = best.max(len);
            }
            if best >= MIN_MATCH {
                covered += best;
                i += best;
                continue;
            }
        }
        i += 1;
    }
    covered
}
