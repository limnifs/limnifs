//! CSV/text categorizer — routes CSV/JSON/TSV to FSST+Brotli.
//!
//! **Status:** DETECTION READY, ROUTING DISABLED.
//!
//! FSST (Fast Static Symbol Table) is a preprocessor that finds the
//! most common substrings in a block and replaces each with a single
//! byte. Reported 1.2–1.5× ratio improvement on text-heavy workloads
//! (CSV columns, JSON keys, log files).
//!
//! This categorizer detects CSV/JSON/TSV by file extension and
//! content sniffing. When `omnizip-fsst` ships and the
//! `limnifs-core::codec::fsst_brotli` composite codec is wired in,
//! flip `FSST_ENABLED` to `true` and the categorizer will claim
//! these files.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::path::Path;

use super::{Categorization, FileCategorizer};
use limnifs_core::codec::CODEC_FSST_BROTLI;

/// Flip to `false` if FSST starts hurting a workload. Currently
/// always on because `omnizip-fsst` 0.4 is shipped and the composite
/// codec at `limnifs-core::codec::fsst_brotli` falls back to plain
/// Brotli when FSST doesn't help.
const FSST_ENABLED: bool = true;

/// Minimum size to bother routing through FSST. Below this, FSST's
/// dictionary overhead exceeds the gain.
const MIN_FSST_SIZE: usize = 4 * 1024;

pub struct CsvTextCategorizer;

impl FileCategorizer for CsvTextCategorizer {
    fn name(&self) -> &'static str {
        "csv-text"
    }

    fn categories(&self) -> &'static [&'static str] {
        &["csv-text/composite"]
    }

    fn categorize(&self, path: &Path, data: &[u8]) -> Option<Categorization> {
        if !FSST_ENABLED {
            return None;
        }
        if data.len() < MIN_FSST_SIZE {
            return None;
        }
        if !looks_like_csv_text(path, data) {
            return None;
        }
        Some(Categorization {
            codec_id: CODEC_FSST_BROTLI,
            codec_params: Vec::new(),
            category: "csv-text/composite",
        })
    }
}

/// Heuristic: does this file look like CSV/JSON/TSV?
///
/// **Extension-required.** Content sniffing alone is too unreliable —
/// PHP/Python/JS source has plenty of commas, quotes, and braces,
/// which trips a pure-content heuristic and routes gigabytes of
/// source code through FSST (extremely slow, marginal ratio gain).
/// The categorizer only claims files with a recognised extension
/// AND a printable majority AND CSV/JSON punctuation present.
#[must_use]
fn looks_like_csv_text(path: &Path, data: &[u8]) -> bool {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_ascii_lowercase(),
        None => return false,
    };
    if !matches!(ext.as_str(), "csv" | "tsv" | "json" | "jsonl" | "ndjson") {
        return false;
    }
    let sample = if data.len() > 4096 { &data[..4096] } else { data };
    let printables = sample
        .iter()
        .filter(|&&b| (0x20..=0x7E).contains(&b) || b == b'\n' || b == b'\r' || b == b'\t')
        .count();
    if (printables as f32 / sample.len() as f32) < 0.95 {
        return false;
    }
    let punct_count = sample
        .iter()
        .filter(|&&b| matches!(b, b',' | b'"' | b'{' | b'}' | b'[' | b']'))
        .count();
    (punct_count as f32 / sample.len() as f32) > 0.01
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn csv_extension_routes_when_enabled() {
        let c = CsvTextCategorizer;
        let csv = "a,b,c\n".repeat(2000);
        let cat = c.categorize(&PathBuf::from("/x.csv"), csv.as_bytes())
            .expect("csv extension claims");
        assert_eq!(cat.codec_id, limnifs_core::codec::CODEC_FSST_BROTLI);
    }

    #[test]
    fn csv_extension_detected_when_enabled() {
        // Toggle the const manually via a re-evaluation would require
        // a feature flag; for now just verify the heuristic.
        let csv = "a,b,c\n".repeat(2000);
        assert!(looks_like_csv_text(&PathBuf::from("/x.csv"), csv.as_bytes()));
        assert!(looks_like_csv_text(&PathBuf::from("/x.json"), csv.as_bytes()));
    }

    #[test]
    fn binary_data_not_misdetected() {
        // Use high-entropy random-ish bytes rather than (0..255).cycle(),
        // which includes all printable ASCII and trips the heuristic.
        let mut state: u64 = 0xDEAD_BEEF_CAFE_BABE;
        let mut binary = Vec::with_capacity(8192);
        for _ in 0..8192 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            binary.push(u8::try_from(state & 0xFF).unwrap());
        }
        assert!(!looks_like_csv_text(&PathBuf::from("/x.csv"), &binary));
    }

    #[test]
    fn empty_extension_does_not_trigger_fsst() {
        // Without a recognised extension, we DON'T claim the file —
        // content sniffing alone is too unreliable (catches source
        // code, emails, etc.).
        let csv = "alpha,beta,gamma\n".repeat(500);
        assert!(!looks_like_csv_text(&PathBuf::from("/noext"), csv.as_bytes()));
    }
}
