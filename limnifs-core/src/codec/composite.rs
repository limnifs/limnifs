//! Composite-codec helpers — filter + inner-codec pipeline.
//!
//! Composite codecs apply a reversible [`Filter`](omnizip_filters::Filter)
//! (byte-shuffle, bit-shuffle, BCJ-x86, BCJ-ARM64, …) to the plaintext
//! before compressing with an inner codec (LZ4, ZSTD, …). Decompression
//! is the exact inverse.
//!
//! ## Wire format
//!
//! ```text
//! [filtered_len: u32 LE][inner_codec_bytes...]
//! ```
//!
//! The 4-byte length prefix lets the inner codec's decoder validate
//! its output length without knowing the original plaintext length
//! (filters are byte-preserving but not length-changing).
//!
//! ## Why this is shared
//!
//! Every composite codec (`shuffle+lz4`, `bitshuffle+lz4`,
//! `shuffle+zstd`, `bcj-x86+lz4`, `bcj-x86+zstd`, `bcj-arm64+lz4`,
//! `bcj-arm64+zstd`) implements the same two-step pipeline. Extracting
//! the pipeline into a single helper removes 7 × ~20 lines of
//! duplication and makes the wire format canonical.
//!
//! ## Adding a new composite
//!
//! 1. Pick a `Filter` impl (existing or new).
//! 2. Pick an inner codec id (`CODEC_LZ4`, `CODEC_ZSTD`, …).
//! 3. Write a 10-line `Codec` impl that delegates `compress`/`decompress`
//!    to [`filter_then_compress`] / [`decompress_then_filter`].
//!
//! See `limnifs-core/src/codec/bcj_composites.rs` and
//! `limnifs-core/src/codec/shuffle_lz4.rs` for examples.

use omnizip_filters::Filter;

use crate::codec::{compress, decompress, CoreError};

/// Apply `filter.encode(plaintext)` then `compress(inner_codec, ...)`,
/// prefixing the result with the filtered-byte count so the decoder
/// can validate the inner codec's output length.
pub fn filter_then_compress<F: Filter>(
    plaintext: &[u8],
    filter: &F,
    inner_codec: u8,
) -> Result<Vec<u8>, CoreError> {
    let filtered = filter.encode(plaintext);
    let inner = compress(inner_codec, &filtered)?;
    let filtered_len = u32::try_from(filtered.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(4 + inner.len());
    out.extend_from_slice(&filtered_len.to_le_bytes());
    out.extend_from_slice(&inner);
    Ok(out)
}

/// Read the 4-byte length prefix, decompress the inner codec, then
/// apply `filter.decode`. The inverse of [`filter_then_compress`].
pub fn decompress_then_filter<F: Filter>(
    compressed: &[u8],
    filter: &F,
    inner_codec: u8,
    error_label: &str,
) -> Result<Vec<u8>, CoreError> {
    if compressed.len() < 4 {
        return Err(CoreError::Corrupt {
            reason: format!("{error_label}: input too short for length prefix"),
        });
    }
    let filtered_len =
        u32::from_le_bytes([compressed[0], compressed[1], compressed[2], compressed[3]]);
    let inner_bytes = &compressed[4..];
    let filtered = decompress(inner_codec, inner_bytes, filtered_len)?;
    Ok(filter.decode(&filtered))
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnizip_filters::shuffle::ByteShuffle;

    #[test]
    fn round_trips_through_any_filter() {
        let plaintext = b"hello world. hello world. hello world.".repeat(10);
        let filter = ByteShuffle::new(4);
        let compressed =
            filter_then_compress(&plaintext, &filter, crate::codec::CODEC_LZ4).expect("compress");
        let recovered =
            decompress_then_filter(&compressed, &filter, crate::codec::CODEC_LZ4, "test")
                .expect("decompress");
        assert_eq!(recovered, plaintext);
    }
}
