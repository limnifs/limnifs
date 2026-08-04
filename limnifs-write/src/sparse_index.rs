//! Cross-image sparse index — Bloom filter of DropIds for fast
//! "is this drop already in image X?" lookups.
//!
//! **Status:** opt-in. Standalone today (no writer integration); a
//! follow-up wires it into the writer so re-compression can skip
//! drops already present in a referenced image.
//!
//! ## Algorithm
//!
//! - Sidecar file `<image>.sparse` containing a Bloom filter of
//!   the image's DropId set (truncated to first 16 bytes; FPP
//!   configurable).
//! - `SparseIndexWriter::insert(DropId)`, `SparseIndexReader::probably_contains(DropId)`.
//! - False positives cost one extra slab read; acceptable.
//!
//! ## Why Bloom filter
//!
//! A 1M-drop image's index is ~1.2 MB (≈ 10 bits per drop at FPP
//! 1%). Fits in L2 cache; queries are sub-microsecond.
//!
//! ## Activation
//!
//! Opt-in via the `sparse-index` feature flag (default off). When
//! enabled, the writer can be configured to emit `<image>.sparse`
//! alongside `<image>.lim`. No-op for default builds.
//!
//! See `TODO.impl/04-writer-pipeline/04-cross-image-sparse-index.md`.

#![cfg(feature = "sparse-index")]

use std::collections::HashSet;
use std::path::Path;

/// Default false-positive probability: 1% (good balance of
/// memory cost vs lookup accuracy).
pub const DEFAULT_FPP: f64 = 0.01;

/// Default bits per DropId. 10 bits at FPP 1%.
const DEFAULT_BITS_PER_ENTRY: usize = 10;

/// Number of hash functions to use for the given target FPP.
/// `k = -log2(fpp)` rounded up. At 1% FPP, k=7.
fn optimal_k(fpp: f64) -> usize {
    let log2 = (fpp.ln() / std::f64::consts::LN_2).abs();
    log2.ceil() as usize
}

/// Number of bits needed for `n` entries at the target FPP.
/// Formula: m = -n * ln(p) / (ln(2))^2.
fn optimal_bits(n: usize, fpp: f64) -> usize {
    if n == 0 {
        return 0;
    }
    let bits = (-(n as f64 * fpp.ln()) / std::f64::consts::LN_2.powi(2)).ceil();
    bits.max(8.0) as usize
}

/// Hash a DropId (first 16 bytes) into two u64 halves. Used to
/// derive k independent hash functions via the standard
/// "double hashing" trick: h_i(x) = h1(x) + i * h2(x).
///
/// The raw bytes of the DropId are mixed with splitmix64 to
/// avoid pathological inputs (e.g. all-zero bytes 8..15 make h2=0
/// which collapses all k hashes to one).
fn hash_pair(drop_id: &[u8; 32]) -> (u64, u64) {
    let mut arr1 = [0u8; 8];
    arr1.copy_from_slice(&drop_id[..8]);
    let mut arr2 = [0u8; 8];
    arr2.copy_from_slice(&drop_id[8..16]);
    let raw1 = u64::from_le_bytes(arr1);
    let raw2 = u64::from_le_bytes(arr2);
    // Mix to avoid h2 == 0 on sequential inputs.
    let h1 = splitmix64(raw1);
    let h2 = splitmix64(raw2.wrapping_add(raw1.rotate_left(31)));
    (h1, h2)
}

/// Deterministic mixing function (from the SplitMix64 PRNG).
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z2 = z;
    z2 = (z2 ^ (z2 >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z2 = (z2 ^ (z2 >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z2 ^ (z2 >> 31)
}

/// Builds a Bloom filter over a set of DropIds.
pub struct SparseIndexWriter {
    bits: Vec<u8>,
    num_bits: usize,
    k: usize,
    fpp: f64,
    entry_count: usize,
}

impl SparseIndexWriter {
    /// Construct a writer sized for `expected_entries` at the
    /// given false-positive probability.
    #[must_use]
    pub fn new(expected_entries: usize, fpp: f64) -> Self {
        let num_bits = optimal_bits(expected_entries, fpp);
        let k = optimal_k(fpp);
        let num_bytes = num_bits.div_ceil(8);
        Self {
            bits: vec![0u8; num_bytes],
            num_bits,
            k,
            fpp,
            entry_count: 0,
        }
    }

    /// Insert a DropId into the filter.
    pub fn insert(&mut self, drop_id: &[u8; 32]) {
        if self.num_bits == 0 {
            return;
        }
        let (h1, h2) = hash_pair(drop_id);
        for i in 0..self.k {
            let combined = h1.wrapping_add((i as u64).wrapping_mul(h2));
            let bit = (combined as usize) % self.num_bits;
            let byte_idx = bit / 8;
            let bit_idx = bit % 8;
            self.bits[byte_idx] |= 1 << bit_idx;
        }
        self.entry_count += 1;
    }

    /// Insert every DropId from `set`. Convenience for the
    /// common "build filter from a SlabStore's index" case.
    pub fn insert_all(&mut self, set: &HashSet<[u8; 32]>) {
        for d in set {
            self.insert(d);
        }
    }

    /// Number of DropIds inserted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entry_count
    }

    /// True iff no DropIds have been inserted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entry_count == 0
    }

    /// Serialise the filter to bytes. Wire format:
    /// - u32 LE: num_bits
    /// - u32 LE: k
    /// - u32 LE: entry_count
    /// - u64 LE: fpp (IEEE 754 double)
    /// - bytes: filter bitmap (ceil(num_bits/8) bytes)
    #[must_use]
    pub fn serialise(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(20 + self.bits.len());
        out.extend_from_slice(&(self.num_bits as u32).to_le_bytes());
        out.extend_from_slice(&(self.k as u32).to_le_bytes());
        out.extend_from_slice(&(self.entry_count as u32).to_le_bytes());
        out.extend_from_slice(&self.fpp.to_le_bytes());
        out.extend_from_slice(&self.bits);
        out
    }

    /// Write the index to `<path>.sparse` (or override).
    ///
    /// # Errors
    /// Returns [`std::io::Error`] on write failure.
    pub fn write_to_file(&self, path: &Path) -> std::io::Result<()> {
        let bytes = self.serialise();
        std::fs::write(path, bytes)
    }
}

/// Reads a Bloom filter from disk and answers `probably_contains`.
pub struct SparseIndexReader {
    bits: Vec<u8>,
    num_bits: usize,
    k: usize,
    #[allow(dead_code)]
    entry_count: usize,
    #[allow(dead_code)]
    fpp: f64,
}

impl SparseIndexReader {
    /// Parse a serialised filter.
    ///
    /// # Errors
    /// Returns `None` if the bytes are too short or malformed.
    #[must_use]
    pub fn deserialise(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 20 {
            return None;
        }
        let mut arr = [0u8; 4];
        arr.copy_from_slice(&bytes[0..4]);
        let num_bits = u32::from_le_bytes(arr) as usize;
        arr.copy_from_slice(&bytes[4..8]);
        let k = u32::from_le_bytes(arr) as usize;
        arr.copy_from_slice(&bytes[8..12]);
        let entry_count = u32::from_le_bytes(arr) as usize;
        let mut arr8 = [0u8; 8];
        arr8.copy_from_slice(&bytes[12..20]);
        let fpp = f64::from_le_bytes(arr8);
        let expected_bytes = num_bits.div_ceil(8);
        if bytes.len() < 20 + expected_bytes {
            return None;
        }
        let bits = bytes[20..20 + expected_bytes].to_vec();
        Some(Self {
            bits,
            num_bits,
            k,
            entry_count,
            fpp,
        })
    }

    /// Load from `<path>.sparse`.
    ///
    /// # Errors
    /// Returns `None` if the file can't be read or is malformed.
    #[must_use]
    pub fn from_file(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        Self::deserialise(&bytes)
    }

    /// Probably-contains query. False positives are possible;
    /// false negatives are not. Returns `false` for an empty
    /// filter (zero entries inserted).
    #[must_use]
    pub fn probably_contains(&self, drop_id: &[u8; 32]) -> bool {
        if self.num_bits == 0 {
            return false;
        }
        let (h1, h2) = hash_pair(drop_id);
        for i in 0..self.k {
            let combined = h1.wrapping_add((i as u64).wrapping_mul(h2));
            let bit = (combined as usize) % self.num_bits;
            let byte_idx = bit / 8;
            let bit_idx = bit % 8;
            if self.bits[byte_idx] & (1 << bit_idx) == 0 {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_inserts_then_reader_finds() {
        let mut writer = SparseIndexWriter::new(100, 0.01);
        let drop_ids: Vec<[u8; 32]> = (0..50)
            .map(|i| {
                let mut arr = [0u8; 32];
                arr[..8].copy_from_slice(&(i as u64).to_le_bytes());
                arr
            })
            .collect();
        for d in &drop_ids {
            writer.insert(d);
        }
        let bytes = writer.serialise();
        let reader = SparseIndexReader::deserialise(&bytes).expect("parse");
        // All inserted drops should be found (no false negatives).
        for d in &drop_ids {
            assert!(
                reader.probably_contains(d),
                "drop {:?} should be present",
                &d[..4]
            );
        }
    }

    #[test]
    fn empty_reader_returns_false_for_anything() {
        let writer = SparseIndexWriter::new(0, 0.01);
        let bytes = writer.serialise();
        let reader = SparseIndexReader::deserialise(&bytes).expect("parse");
        assert!(!reader.probably_contains(&[0u8; 32]));
    }

    #[test]
    fn fpp_within_bounds() {
        // 1000 entries, 1% FPP target. Test 10000 random drops not
        // in the set; expect roughly 1% false positive rate.
        let mut writer = SparseIndexWriter::new(1000, 0.01);
        let in_set: HashSet<[u8; 32]> = (0..1000)
            .map(|i| {
                let mut arr = [0u8; 32];
                arr[..8].copy_from_slice(&(i as u64).to_le_bytes());
                arr
            })
            .collect();
        writer.insert_all(&in_set);
        let bytes = writer.serialise();
        let reader = SparseIndexReader::deserialise(&bytes).expect("parse");
        let mut false_positives = 0;
        let total_tests = 10_000;
        for i in 0..total_tests {
            let mut arr = [0u8; 32];
            arr[..8].copy_from_slice(&(u64::MAX - i).to_le_bytes());
            if !in_set.contains(&arr) && reader.probably_contains(&arr) {
                false_positives += 1;
            }
        }
        let observed_fpp = false_positives as f64 / total_tests as f64;
        // 5% upper bound — Bloom filters have variance, so a hard
        // 1% bound would be flaky. 5x is empirically safe.
        assert!(
            observed_fpp < 0.05,
            "observed FPP {observed_fpp:.4} exceeds 5% bound"
        );
    }

    #[test]
    fn round_trips_via_file() {
        let temp = std::env::temp_dir().join(format!(
            "limnifs-sparse-rt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
        ));
        let mut writer = SparseIndexWriter::new(100, 0.01);
        writer.insert(&[1u8; 32]);
        writer.insert(&[2u8; 32]);
        writer.write_to_file(&temp).expect("write");
        let reader = SparseIndexReader::from_file(&temp).expect("read");
        assert!(reader.probably_contains(&[1u8; 32]));
        assert!(reader.probably_contains(&[2u8; 32]));
        assert!(!reader.probably_contains(&[3u8; 32]));
        let _ = std::fs::remove_file(&temp);
    }
}
