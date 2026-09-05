//! Boundary-identical parallel FastCDC (two-phase scan + replay).
//!
//! The scalar [`FastCDC`] is a serial gear-hash roll: one core walks
//! every byte of every large file. This strategy parallelises that
//! walk WITHOUT changing a single boundary — images pack bit-for-bit
//! identically to the scalar chunker, so it needs no format bump, no
//! feature flag, and no opt-in.
//!
//! ## Where the fingerprint is (and isn't) positional
//!
//! FastCDC resets its fingerprint at each chunk start and skips the
//! min-size region WITHOUT hashing — the fold begins at
//! `chunk_start + min_size`, not at `chunk_start`. Write `w` for the
//! number of folded bytes at a tested position (`w = pos -
//! fold_start + 1`):
//!
//! - `w < 64`: the value depends on the fold start, i.e. on where
//!   the PREVIOUS boundary landed — inherently sequential.
//! - `w >= 64`: the gear hash `fp = (fp << 1) + gear[b]` over `u64`
//!   has an effective window of 64 bytes, so the value equals the
//!   fold over the trailing 64 bytes of the stream alone.
//!
//! That split drives the whole design: the sequential prefix region
//! is tiny (at most 63 positions per chunk), while the windowed
//! region — the vast majority of every large file — parallelises
//! exactly.
//!
//! ## Two phases (MECE: discovery vs decision)
//!
//! - **Phase A — candidate scan (parallel, memory-bound).** Lanes
//!   roll the exact windowed fingerprint over disjoint regions
//!   (priming each lane 64 bytes before its start) and record
//!   `(position, fp)` pairs where `fp & probe_mask == 0`, with
//!   `probe_mask = mask1 & mask2`. Both real fire conditions imply
//!   the probe, so the candidate list is a strict superset of every
//!   boundary in the windowed region. With the default sizes the
//!   probe is 17 bits — roughly one candidate per 128 KiB.
//! - **Phase B — boundary replay (serial).** For each chunk: a
//!   ≤63-byte micro-scan recomputes the prefix-dependent folds from
//!   `chunk_start + min_size` (the sequential region), then the
//!   exact FastCDC decision rule — min-size skip, mask1/mask2 split
//!   by average, forced max — walks the candidates, consulting
//!   fingerprints instead of recomputing them.
//!
//! ## Scheduling
//!
//! Lanes run as nested rayon work on the global pool — the same
//! work-stealing shape the writer already uses for per-chunk BLAKE3
//! hashing inside pipeline workers. Safe here because the streaming
//! producer thread is pool-free (see `write_directory_streaming`).

use rayon::prelude::*;

use super::{Chunker, FastCDC};

/// The gear hash's effective window: each byte shifts out of the
/// u64 fingerprint after this many successors.
const GEAR_WINDOW: usize = 64;

/// Inputs at or above this size take the two-phase path; smaller
/// ones delegate to the scalar roll (lane setup would cost more
/// than the scan saves). With the default 256 KiB average chunk
/// this is ~32 chunks — enough lanes to matter.
const DEFAULT_PARALLEL_THRESHOLD: usize = 8 * 1024 * 1024;

/// [`FastCDC`] with a parallel, boundary-identical `chunk_slice`.
///
/// Owns the scalar chunker as its reference implementation and
/// small-input fallback; the parallel path is purely a throughput
/// overlay with byte-identical output (pinned by tests here and by
/// the writer's pack-twice determinism suite).
#[derive(Clone, Debug)]
pub struct ParallelFastCDC {
    scalar: FastCDC,
    threshold: usize,
}

impl ParallelFastCDC {
    /// Construct with explicit size parameters and the default
    /// parallel threshold.
    ///
    /// # Errors
    ///
    /// Same contract as [`FastCDC::new`].
    pub fn new(min_size: usize, avg_size: usize, max_size: usize) -> Result<Self, &'static str> {
        Ok(Self {
            scalar: FastCDC::new(min_size, avg_size, max_size)?,
            threshold: DEFAULT_PARALLEL_THRESHOLD,
        })
    }

    /// Wrap an existing scalar chunker, taking large inputs to the
    /// two-phase path once `data.len() >= threshold`.
    #[must_use]
    pub fn with_scalar(scalar: FastCDC, threshold: usize) -> Self {
        Self { scalar, threshold }
    }

    /// The scalar reference implementation this strategy wraps.
    #[must_use]
    pub fn scalar(&self) -> &FastCDC {
        &self.scalar
    }

    /// Phase A: lanes over disjoint regions record probe hits.
    ///
    /// `lanes` is a parameter (not read from the environment) so the
    /// thread-determinism test can pin output equality across lane
    /// counts; callers pass the pool width.
    fn scan_candidates(&self, data: &[u8], lanes: usize) -> Vec<(usize, u64)> {
        let lane_len = data.len().div_ceil(lanes);
        let probe = self.scalar.mask1 & self.scalar.mask2;
        let gear = &self.scalar.gear.bytes;
        (0..lanes)
            .into_par_iter()
            .map(|lane| {
                let r0 = lane * lane_len;
                let r1 = (r0 + lane_len).min(data.len());
                let mut hits = Vec::new();
                if r0 >= r1 {
                    return hits;
                }
                // Prime from r0 - 64 so the fingerprint at r0 is
                // already the exact windowed value (see module docs).
                // Lane 0 primes from the true stream start, which is
                // the windowed value for its region as well.
                let prime_start = r0.saturating_sub(GEAR_WINDOW);
                let mut fp = 0u64;
                for i in prime_start..r1 {
                    fp = (fp << 1).wrapping_add(gear[usize::from(data[i])]);
                    if i >= r0 && fp & probe == 0 {
                        hits.push((i, fp));
                    }
                }
                hits
            })
            .collect::<Vec<Vec<(usize, u64)>>>()
            .concat()
    }

    /// Phase B: serial replay of the exact FastCDC decision rule.
    /// Mirrors `FastCDC::find_boundary` state-for-state: min-size
    /// skip (which also skips hashing), mask split at avg, forced
    /// boundary at max, short tail.
    fn replay_boundaries(
        &self,
        data: &[u8],
        candidates: &[(usize, u64)],
    ) -> Vec<(usize, usize)> {
        let (min_size, avg_size, max_size, mask1, mask2) = (
            self.scalar.min_size,
            self.scalar.avg_size,
            self.scalar.max_size,
            self.scalar.mask1,
            self.scalar.mask2,
        );
        let gear = &self.scalar.gear.bytes;
        let mut chunks = Vec::new();
        let mut start = 0usize;
        let mut cursor = 0usize;
        while start < data.len() {
            let max_end = (start + max_size).min(data.len());
            if max_end - start <= min_size {
                chunks.push((start, max_end));
                start = max_end;
                continue;
            }
            let fold_start = start + min_size;
            // Micro-scan: positions with fewer than 64 folded bytes
            // depend on the fold start, so recompute them here — at
            // most 63 gear steps per chunk.
            let warm_end = (fold_start + GEAR_WINDOW - 1).min(max_end);
            let mut end = max_end;
            let mut fired = false;
            let mut fp = 0u64;
            let mut pos = fold_start;
            while pos < warm_end {
                fp = (fp << 1).wrapping_add(gear[usize::from(data[pos])]);
                if fp & mask_for_pos(pos - start, avg_size, mask1, mask2) == 0 {
                    end = pos + 1;
                    fired = true;
                    break;
                }
                pos += 1;
            }
            if !fired {
                // Windowed region: lane fingerprints are exact from
                // fold_start + 63 (>= 64 folded bytes) onward.
                let candidate_from = fold_start + GEAR_WINDOW - 1;
                for &(cpos, cfp) in &candidates[cursor..] {
                    if cpos >= max_end {
                        break;
                    }
                    if cpos < candidate_from {
                        continue;
                    }
                    if cfp & mask_for_pos(cpos - start, avg_size, mask1, mask2) == 0 {
                        end = cpos + 1;
                        break;
                    }
                }
            }
            chunks.push((start, end));
            // Every candidate before the new boundary is below the
            // next chunk's windowed region — retire them for good.
            while cursor < candidates.len() && candidates[cursor].0 < end {
                cursor += 1;
            }
            start = end;
        }
        chunks
    }

    /// Two-phase split for inputs above the threshold. Exposed with
    /// an explicit lane count for the determinism test.
    fn chunk_slice_with_lanes<'a>(&self, data: &'a [u8], lanes: usize) -> Vec<&'a [u8]> {
        let candidates = self.scan_candidates(data, lanes);
        self.replay_boundaries(data, &candidates)
            .into_iter()
            .map(|(s, e)| &data[s..e])
            .collect()
    }
}

/// The FastCDC mask for a tested position: level-1 before the
/// average size, level-2 after.
fn mask_for_pos(pos_in_chunk: usize, avg_size: usize, mask1: u64, mask2: u64) -> u64 {
    if pos_in_chunk < avg_size {
        mask1
    } else {
        mask2
    }
}

impl Default for ParallelFastCDC {
    fn default() -> Self {
        Self {
            scalar: FastCDC::default(),
            threshold: DEFAULT_PARALLEL_THRESHOLD,
        }
    }
}

impl Chunker for ParallelFastCDC {
    fn chunk_slice<'a>(&self, data: &'a [u8]) -> Vec<&'a [u8]> {
        if data.len() < self.threshold || rayon::current_num_threads() < 2 {
            return self.scalar.chunk_slice(data);
        }
        self.chunk_slice_with_lanes(data, rayon::current_num_threads())
    }

    fn chunk_reader(&self, reader: &mut dyn std::io::Read) -> std::io::Result<Vec<Vec<u8>>> {
        // Streaming keeps its constant-memory contract: the
        // two-phase path needs the whole buffer for random lane
        // access, so pipes stay on the scalar roll.
        self.scalar.chunk_reader(reader)
    }

    fn avg_chunk_size(&self) -> usize {
        self.scalar.avg_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pseudo_random_bytes(seed: u64, count: usize) -> Vec<u8> {
        let mut state = seed;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            out.push(u8::try_from(state >> 56).expect("fits u8"));
        }
        out
    }

    fn boundaries(chunks: &[&[u8]]) -> Vec<usize> {
        let mut out = Vec::with_capacity(chunks.len() + 1);
        out.push(0);
        let mut acc = 0;
        for c in chunks {
            acc += c.len();
            out.push(acc);
        }
        out
    }

    /// Small sizes with a tiny threshold exercise both phases
    /// densely; equality with the scalar roll is the whole contract.
    fn subject(threshold: usize) -> ParallelFastCDC {
        ParallelFastCDC::with_scalar(
            FastCDC::new(64, 256, 1024).expect("valid sizes"),
            threshold,
        )
    }

    #[test]
    fn parallel_matches_scalar_on_random_input() {
        let chunker = subject(1024);
        let data = pseudo_random_bytes(0xBEEF, 512 * 1024);
        for lanes in [1usize, 2, 3, 5, 8, 16] {
            assert_eq!(
                boundaries(&chunker.chunk_slice_with_lanes(&data, lanes)),
                boundaries(&chunker.scalar().chunk_slice(&data)),
                "lane count {lanes} must be boundary-identical"
            );
        }
    }

    #[test]
    fn parallel_matches_scalar_on_structured_input() {
        let chunker = subject(1024);
        // Low-entropy + periodic content: forces long mask2 regions
        // and forced-max boundaries the replay must reproduce.
        let mut data = Vec::with_capacity(400 * 1024);
        for block in 0..400 {
            match block % 3 {
                0 => data.extend(std::iter::repeat(0x00).take(1024)),
                1 => data.extend(std::iter::repeat(0xFF).take(1024)),
                _ => data.extend((0..1024u32).map(|i| u8::try_from(i & 0xFF).expect("fits"))),
            }
        }
        for lanes in [2usize, 4, 7] {
            assert_eq!(
                boundaries(&chunker.chunk_slice_with_lanes(&data, lanes)),
                boundaries(&chunker.scalar().chunk_slice(&data)),
                "structured input, lanes {lanes}"
            );
        }
    }

    #[test]
    fn parallel_matches_scalar_with_narrow_min_size() {
        // min_size < 64: chunk folds start closer together, so the
        // prefix-dependent region dominates more chunks — the
        // micro-scan must still reproduce every boundary.
        let chunker = ParallelFastCDC::with_scalar(
            FastCDC::new(8, 64, 256).expect("valid sizes"),
            1,
        );
        let data = pseudo_random_bytes(21, 128 * 1024);
        for lanes in [2usize, 5, 9] {
            assert_eq!(
                boundaries(&chunker.chunk_slice_with_lanes(&data, lanes)),
                boundaries(&chunker.scalar().chunk_slice(&data)),
                "narrow min, lanes {lanes}"
            );
        }
    }

    #[test]
    fn parallel_matches_scalar_at_default_sizes() {
        // The shipped configuration: 64 KiB / 256 KiB / 1 MiB.
        let chunker = ParallelFastCDC::with_scalar(FastCDC::default(), 1024);
        let data = pseudo_random_bytes(0x5EED, 6 * 1024 * 1024);
        assert_eq!(
            boundaries(&chunker.chunk_slice_with_lanes(&data, 4)),
            boundaries(&chunker.scalar().chunk_slice(&data)),
            "default sizes must be boundary-identical"
        );
    }

    #[test]
    fn parallel_covers_input_exactly() {
        let chunker = subject(1024);
        let data = pseudo_random_bytes(7, 300 * 1024);
        let chunks = chunker.chunk_slice_with_lanes(&data, 4);
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, data.len());
        assert_eq!(chunks.first().map(|c| c.as_ptr()), Some(data.as_ptr()));
    }

    #[test]
    fn below_threshold_delegates_to_scalar() {
        let chunker = subject(1024 * 1024);
        let data = pseudo_random_bytes(3, 4096);
        assert_eq!(
            boundaries(&chunker.chunk_slice(&data)),
            boundaries(&chunker.scalar().chunk_slice(&data))
        );
    }

    #[test]
    fn empty_and_short_inputs_match_scalar() {
        let chunker = subject(1);
        assert!(chunker.chunk_slice(&[]).is_empty());
        let data = vec![0xAB; 50];
        let chunks = chunker.chunk_slice(&data);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 50);
    }

    #[test]
    fn chunk_reader_stays_streaming_and_matches() {
        use std::io::Cursor;
        let chunker = subject(1);
        let data = pseudo_random_bytes(99, 100 * 1024);
        let mut cursor = Cursor::new(&data);
        let streamed = chunker.chunk_reader(&mut cursor).expect("read succeeds");
        let sliced: Vec<Vec<u8>> = chunker
            .chunk_slice(&data)
            .into_iter()
            .map(Vec::from)
            .collect();
        assert_eq!(streamed, sliced);
    }
}
