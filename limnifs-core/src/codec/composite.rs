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

use crate::codec::{compress, decompress, Codec, CoreError};

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

/// A complete composite codec: one reversible [`Filter`] plus one
/// inner codec, wired through the shared [`filter_then_compress`] /
/// [`decompress_then_filter`] pipeline. This is the ONLY `Codec`
/// implementation for composites — the seven shipped composites
/// (shuffle+lz4, shuffle+zstd, bitshuffle+lz4, and the four BCJ
/// variants) are instances, not hand-rolled impls (TODO.remaining
/// item 2: deletes the per-codec boilerplate and the BCJ macro).
///
/// The wire format is exactly the pipeline's: a 4-byte filtered-length
/// prefix + inner-codec bytes; filters that are self-describing
/// (shuffle) carry their own config inside the filtered stream.
pub struct FilterCodecComposite<F: Filter> {
    filter: F,
    inner: u8,
    id: u8,
    name: &'static str,
    min_compress_size: usize,
}
impl<F: Filter> FilterCodecComposite<F> {
    /// Assemble a composite. `min_compress_size` gates the
    /// tournament from wasting the filter pass on tiny inputs.
    #[must_use]
    pub const fn new(
        filter: F,
        inner: u8,
        id: u8,
        name: &'static str,
        min_compress_size: usize,
    ) -> Self {
        Self {
            filter,
            inner,
            id,
            name,
            min_compress_size,
        }
    }
}

impl<F: Filter> Codec for FilterCodecComposite<F> {
    fn id(&self) -> u8 {
        self.id
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn min_compress_size(&self) -> usize {
        self.min_compress_size
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        filter_then_compress(plaintext, &self.filter, self.inner)
    }

    fn decompress(&self, compressed: &[u8], _expected_len: u32) -> Result<Vec<u8>, CoreError> {
        decompress_then_filter(compressed, &self.filter, self.inner, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Codec;
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

    /// IMPL-9 acceptance: every composite codec registered in the
    /// default registry round-trips through the shared pipeline —
    /// proof that collapsing the seven hand-rolled impls (and the
    /// BCJ macro) into `FilterCodecComposite` instances changed no
    /// codec behavior.
    #[test]
    fn all_registered_composites_round_trip_through_the_pipeline() {
        let composite_ids = [
            crate::codec::CODEC_BLOSC2_SHUFFLE_LZ4, // 0x0A shuffle+lz4
            crate::codec::CODEC_SHUFFLE_ZSTD,       // 0x0E shuffle+zstd
            crate::codec::CODEC_BITSHUFFLE_LZ4,     // 0x0F bitshuffle+lz4
            crate::codec::CODEC_BCJ_X86_LZ4,        // 0x20
            crate::codec::CODEC_BCJ_X86_ZSTD,       // 0x21
            crate::codec::CODEC_BCJ_ARM64_LZ4,      // 0x23
            crate::codec::CODEC_BCJ_ARM64_ZSTD,     // 0x24
        ];
        let registry = crate::codec::default_registry();
        // Numeric-array-ish fixture: shuffles see correlated bytes;
        // BCJ passes through when it finds no relative calls.
        let mut payload = Vec::with_capacity(8 * 1024);
        let mut v = 0.25f32;
        for i in 0..2048 {
            v = (v + i as f32 * 0.001).sin();
            payload.extend_from_slice(&v.to_le_bytes());
        }
        for id in composite_ids {
            let compressed = registry
                .compress(id, &payload)
                .unwrap_or_else(|e| panic!("0x{id:02X} compress: {e}"));
            let recovered = registry
                .decompress(id, &compressed, payload.len() as u32)
                .unwrap_or_else(|e| panic!("0x{id:02X} decompress: {e}"));
            assert_eq!(recovered, payload, "0x{id:02X} round-trip");
        }
    }

    /// The wire format is the shared pipeline for every composite:
    /// 4-byte filtered-length prefix + inner codec bytes.
    #[test]
    fn composite_wire_format_is_the_shared_pipeline() {
        let codec = FilterCodecComposite::new(
            ByteShuffle::new(4),
            crate::codec::CODEC_LZ4,
            0x0A,
            "shuffle+lz4-test",
            512,
        );
        let payload: Vec<u8> = (0..4096u32).flat_map(|i| i.to_le_bytes()).collect();
        let via_codec = codec.compress(&payload).expect("codec path");
        let via_pipeline =
            filter_then_compress(&payload, &ByteShuffle::new(4), crate::codec::CODEC_LZ4)
                .expect("pipeline path");
        assert_eq!(via_codec, via_pipeline, "codec output == pipeline output");
    }
}
