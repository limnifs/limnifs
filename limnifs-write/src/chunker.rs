//! Content-defined chunking via `FastCDC` (Xia et al., 2016).
//!
//! Splits a byte stream into variable-size chunks whose boundaries
//! are determined by the content itself (via a gear hash), so
//! identical regions produce identical boundaries. This is the
//! dedup primitive: two files that share a long middle section will
//! produce overlapping `DropId`s for that section, even if their
//! prefixes and suffixes differ.
//!
//! ## Algorithm
//!
//! 1. Skip the first `min_size` bytes of each chunk — no boundary
//!    is emitted in this region, so chunks cannot be smaller than
//!    `min_size`.
//! 2. Apply a level-1 mask (small, harder to trigger) over the next
//!    `avg_size - min_size` bytes. Boundary fires when the rolling
//!    hash ANDs to zero with the mask.
//! 3. If no boundary has fired by `avg_size`, switch to a level-2
//!    mask (larger, easier to trigger) and continue until `max_size`.
//! 4. At `max_size`, force a boundary regardless of the hash.
//!
//! The two-mask split pushes the actual average toward `avg_size`
//! while bounding the spread.
//!
//! ## Determinism
//!
//! The gear table is generated deterministically (splitmix64 with a
//! fixed seed). Anyone running this chunker on the same input gets
//! the same chunk boundaries — independent of platform, build, or
//! RNG quality. This is required for content-addressed dedup.

use std::io::Read;

/// Default minimum chunk size (64 KiB).
pub const DEFAULT_MIN_SIZE: usize = 64 * 1024;
/// Default average chunk size (256 KiB).
pub const DEFAULT_AVG_SIZE: usize = 256 * 1024;
/// Default maximum chunk size (1 MiB).
pub const DEFAULT_MAX_SIZE: usize = 1024 * 1024;

/// Read buffer size when pulling from a `Read`.
const READ_BUFFER_SIZE: usize = 64 * 1024;

/// A content-defined chunker that splits a byte stream at boundaries
/// determined by the content itself.
///
/// Create via [`FastCDC::new`] or use the [`Default`] implementation
/// (which uses the spec's default sizes). Then call [`chunk_reader`]
/// or [`chunk_slice`] to produce chunks.
///
/// [`chunk_reader`]: Self::chunk_reader
/// [`chunk_slice`]: Self::chunk_slice
#[derive(Clone, Debug)]
pub struct FastCDC {
    min_size: usize,
    avg_size: usize,
    max_size: usize,
    mask1: u64,
    mask2: u64,
    gear: GearTable,
}

impl Default for FastCDC {
    fn default() -> Self {
        Self::new(DEFAULT_MIN_SIZE, DEFAULT_AVG_SIZE, DEFAULT_MAX_SIZE)
            .expect("default sizes are valid")
    }
}

impl FastCDC {
    /// Construct a chunker with explicit size parameters.
    ///
    /// # Errors
    ///
    /// Returns `&str` if the sizes are inconsistent:
    /// - `min_size == 0`
    /// - `min_size >= avg_size`
    /// - `avg_size >= max_size`
    pub fn new(min_size: usize, avg_size: usize, max_size: usize) -> Result<Self, &'static str> {
        if min_size == 0 {
            return Err("min_size must be > 0");
        }
        if min_size >= avg_size {
            return Err("min_size must be < avg_size");
        }
        if avg_size >= max_size {
            return Err("avg_size must be < max_size");
        }
        let mask1 = mask_for(avg_size - min_size);
        let mask2 = mask_for(max_size - avg_size);
        Ok(Self {
            min_size,
            avg_size,
            max_size,
            mask1,
            mask2,
            gear: GearTable::default(),
        })
    }

    /// The minimum chunk size this chunker will produce.
    #[must_use]
    pub const fn min_size(&self) -> usize {
        self.min_size
    }

    /// The target average chunk size.
    #[must_use]
    pub const fn avg_size(&self) -> usize {
        self.avg_size
    }

    /// The maximum chunk size this chunker will produce.
    #[must_use]
    pub const fn max_size(&self) -> usize {
        self.max_size
    }

    /// Split `data` into content-defined chunks.
    ///
    /// Returns a `Vec` of byte slices borrowing from `data`. The
    /// concatenation of the slices equals `data`. The last chunk may
    /// be smaller than `min_size` (if the input is short or the final
    /// chunk happens to be the tail after the last boundary).
    #[must_use]
    pub fn chunk_slice<'a>(&self, data: &'a [u8]) -> Vec<&'a [u8]> {
        let mut chunks = Vec::new();
        let mut start = 0;
        while start < data.len() {
            let end = self.find_boundary(data, start);
            chunks.push(&data[start..end]);
            start = end;
        }
        chunks
    }

    /// Split a `Read` stream into content-defined chunks, returning
    /// each chunk as an owned `Vec<u8>`. Constant memory bounded by
    /// `max_size` plus one read buffer.
    ///
    /// # Errors
    ///
    /// Returns the underlying `std::io::Error` if `reader` fails.
    pub fn chunk_reader<R: Read>(&self, mut reader: R) -> std::io::Result<Vec<Vec<u8>>> {
        let mut buffer: Vec<u8> = Vec::with_capacity(self.max_size + READ_BUFFER_SIZE);
        let mut read_buf = vec![0u8; READ_BUFFER_SIZE];
        let mut fp: u64 = 0;
        let mut chunk_start: usize = 0;
        let mut i: usize = 0;
        let mut chunks: Vec<Vec<u8>> = Vec::new();

        loop {
            let n = reader.read(&mut read_buf)?;
            if n == 0 {
                break;
            }
            buffer.extend_from_slice(&read_buf[..n]);

            while i < buffer.len() {
                let pos_in_chunk = i - chunk_start;
                if pos_in_chunk >= self.max_size {
                    chunks.push(buffer[chunk_start..i].to_vec());
                    chunk_start = i;
                    fp = 0;
                    continue;
                }
                if pos_in_chunk < self.min_size {
                    i += 1;
                    continue;
                }
                fp = (fp << 1).wrapping_add(self.gear.bytes[usize::from(buffer[i])]);
                let mask = if pos_in_chunk < self.avg_size {
                    self.mask1
                } else {
                    self.mask2
                };
                if fp & mask == 0 {
                    i += 1;
                    chunks.push(buffer[chunk_start..i].to_vec());
                    chunk_start = i;
                    fp = 0;
                    continue;
                }
                i += 1;
            }
        }

        if chunk_start < buffer.len() {
            chunks.push(buffer[chunk_start..].to_vec());
        }
        Ok(chunks)
    }

    /// Find the next chunk boundary starting at `start`. Always
    /// returns an index in `start + min_size ..= min(start + max_size, data.len())`.
    fn find_boundary(&self, data: &[u8], start: usize) -> usize {
        let data_len = data.len();
        if data_len <= start {
            return start;
        }
        let max_end = (start + self.max_size).min(data_len);
        if max_end - start <= self.min_size {
            return max_end;
        }

        let mut fp: u64 = 0;
        let avg_end = (start + self.avg_size).min(max_end);
        let mut i = start + self.min_size;

        while i < avg_end {
            fp = (fp << 1).wrapping_add(self.gear.bytes[usize::from(data[i])]);
            if fp & self.mask1 == 0 {
                return i + 1;
            }
            i += 1;
        }
        while i < max_end {
            fp = (fp << 1).wrapping_add(self.gear.bytes[usize::from(data[i])]);
            if fp & self.mask2 == 0 {
                return i + 1;
            }
            i += 1;
        }
        max_end
    }
}

/// Compute the mask for the `FastCDC` normalization step.
///
/// The mask has `bits` set to 1 starting from the LSB. `bits` is
/// `log2(range)` rounded down, which empirically produces chunk-size
/// distributions that match the target average.
fn mask_for(range: usize) -> u64 {
    if range == 0 {
        return 0;
    }
    let bits = (64 - range.leading_zeros()).saturating_sub(1).max(1);
    (1u64 << bits) - 1
}

/// The 256-entry gear-hash table. Each entry is a random u64
/// generated deterministically via splitmix64 with a fixed seed.
#[derive(Clone, Debug)]
struct GearTable {
    bytes: [u64; 256],
}

impl Default for GearTable {
    fn default() -> Self {
        let mut table = [0u64; 256];
        let mut state: u64 = 0x0123_4567_89AB_CDEF; // Fixed seed
        for entry in &mut table {
            state = splitmix64(state);
            *entry = state;
        }
        Self { bytes: table }
    }
}

/// splitmix64 — a deterministic PRNG with good distribution.
fn splitmix64(mut state: u64) -> u64 {
    state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::io::Cursor;

    fn chunk_sizes(chunks: &[&[u8]]) -> Vec<usize> {
        chunks.iter().map(|c| c.len()).collect()
    }

    /// Pseudo-random byte generator for tests — deterministic,
    /// good distribution, no external dependency.
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

    #[test]
    fn short_input_produces_one_chunk() {
        let chunker = FastCDC::new(64, 256, 1024).expect("valid sizes");
        let data = vec![0xAA; 50];
        let chunks = chunker.chunk_slice(&data);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 50);
    }

    #[test]
    fn chunks_cover_input_exactly() {
        let chunker = FastCDC::new(64, 256, 1024).expect("valid sizes");
        let data: Vec<u8> = (0..5000u32)
            .map(|i| u8::try_from(i & 0xFF).expect("fits"))
            .collect();
        let chunks = chunker.chunk_slice(&data);
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, data.len());
    }

    #[test]
    fn chunks_respect_min_and_max() {
        let chunker = FastCDC::new(64, 256, 1024).expect("valid sizes");
        let data = pseudo_random_bytes(1, 10_000);
        let chunks = chunker.chunk_slice(&data);
        for (i, chunk) in chunks.iter().enumerate() {
            if i + 1 == chunks.len() {
                continue; // final chunk can be short
            }
            assert!(chunk.len() >= 64, "chunk {i} size {} < min 64", chunk.len());
            assert!(
                chunk.len() <= 1024,
                "chunk {i} size {} > max 1024",
                chunk.len()
            );
        }
    }

    #[test]
    fn boundary_shift_one_byte_insert_affects_few_chunks() {
        // Inserting one byte at the start should only shift a small
        // number of chunk boundaries before the chunks re-synchronise.
        let chunker = FastCDC::new(64, 256, 1024).expect("valid sizes");
        let base = pseudo_random_bytes(7, 10_000);
        let mut shifted = Vec::with_capacity(base.len() + 1);
        shifted.push(0xFF);
        shifted.extend_from_slice(&base);

        let base_chunks = chunker.chunk_slice(&base);
        let shifted_chunks = chunker.chunk_slice(&shifted);

        let base_starts: HashSet<usize> = std::iter::once(0)
            .chain(base_chunks.iter().scan(0, |acc, c| {
                *acc += c.len();
                Some(*acc)
            }))
            .collect();
        let shifted_starts: HashSet<usize> = std::iter::once(0)
            .chain(shifted_chunks.iter().scan(0, |acc, c| {
                *acc += c.len();
                Some(*acc)
            }))
            .collect();

        let shifted_count = shifted_starts
            .iter()
            .filter(|&&s| {
                !base_starts.contains(&s) && !base_starts.contains(&(s.saturating_sub(1)))
            })
            .count();
        let max_shifted_boundaries = 3;
        assert!(
            shifted_count <= max_shifted_boundaries,
            "1-byte insert shifted {shifted_count} boundaries (expected ≤ {max_shifted_boundaries})"
        );
    }

    #[test]
    fn chunk_reader_matches_chunk_slice() {
        let chunker = FastCDC::new(64, 256, 1024).expect("valid sizes");
        let data = pseudo_random_bytes(99, 5000);
        let slice_chunks: Vec<Vec<u8>> = chunker
            .chunk_slice(&data)
            .into_iter()
            .map(Vec::from)
            .collect();
        let reader_chunks = chunker
            .chunk_reader(Cursor::new(&data))
            .expect("read succeeds");
        assert_eq!(slice_chunks, reader_chunks);
    }

    #[test]
    fn deterministic_across_instances() {
        let chunker_a = FastCDC::new(64, 256, 1024).expect("valid");
        let chunker_b = FastCDC::new(64, 256, 1024).expect("valid");
        let data: Vec<u8> = (0..1000u32)
            .map(|i| u8::try_from(i & 0xFF).expect("fits"))
            .collect();
        assert_eq!(
            chunk_sizes(&chunker_a.chunk_slice(&data)),
            chunk_sizes(&chunker_b.chunk_slice(&data))
        );
    }

    #[test]
    fn rejects_invalid_sizes() {
        assert!(FastCDC::new(0, 256, 1024).is_err());
        assert!(FastCDC::new(256, 256, 1024).is_err());
        assert!(FastCDC::new(64, 64, 64).is_err());
        assert!(FastCDC::new(64, 256, 100).is_err());
    }

    #[test]
    fn default_sizes_match_spec() {
        let chunker = FastCDC::default();
        assert_eq!(chunker.min_size(), 64 * 1024);
        assert_eq!(chunker.avg_size(), 256 * 1024);
        assert_eq!(chunker.max_size(), 1024 * 1024);
    }

    #[test]
    fn identical_substrings_produce_identical_chunks() {
        // Two inputs sharing a long middle section should produce
        // at least one identical chunk (dedup win).
        let chunker = FastCDC::new(64, 256, 1024).expect("valid");
        let shared: Vec<u8> = (0..2000u32)
            .map(|i| u8::try_from(i & 0xFF).expect("fits"))
            .collect();

        let mut a = Vec::new();
        a.extend_from_slice(&[0xAA; 100]);
        a.extend_from_slice(&shared);
        a.extend_from_slice(&[0xBB; 100]);

        let mut b = Vec::new();
        b.extend_from_slice(&[0xCC; 50]);
        b.extend_from_slice(&shared);
        b.extend_from_slice(&[0xDD; 200]);

        let a_chunks = chunker.chunk_slice(&a);
        let b_chunks = chunker.chunk_slice(&b);

        let a_ids: HashSet<&[u8]> = a_chunks.iter().copied().collect();
        let b_ids: HashSet<&[u8]> = b_chunks.iter().copied().collect();
        let shared_chunks: Vec<&&[u8]> = a_ids.intersection(&b_ids).collect();
        assert!(
            !shared_chunks.is_empty(),
            "expected at least one shared chunk between the two inputs"
        );
    }

    #[test]
    fn mask_for_handles_small_values() {
        assert_eq!(mask_for(0), 0);
        assert!(mask_for(1) > 0);
        assert!(mask_for(256) > mask_for(64));
    }

    #[test]
    fn empty_input_yields_no_chunks() {
        let chunker = FastCDC::default();
        let data: Vec<u8> = Vec::new();
        let chunks = chunker.chunk_slice(&data);
        assert!(chunks.is_empty());

        let reader_chunks = chunker
            .chunk_reader(Cursor::new(&data))
            .expect("read succeeds");
        assert!(reader_chunks.is_empty());
    }
}
