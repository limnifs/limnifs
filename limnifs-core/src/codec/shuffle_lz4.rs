//! Shuffle+LZ4 composite codec (id 0x0A): for scientific float data.
//!
//! Applies byte-shuffle (transpose N items × item_size bytes) before
//! LZ4 compression. The shuffle exposes mantissa/exponent correlation
//! in float arrays that LZ4's match finder can then exploit.
//!
//! Typical ratio improvement: float32 arrays go from ~80% (raw LZ4)
//! to ~40% (shuffled LZ4) on smooth scientific data.
//!
//! ## Wire format
//!
//! The shuffle filter is self-describing: its output starts with
//! `[tag: u8][item_size: u8]` so the decoder recovers the item_size
//! without external config. The codec wrapper LZ4-compresses the
//! shuffled bytes:
//!
//! ```text
//! [LZ4 compressed block of: [shuffle_tag][item_size][shuffled_data]]
//! ```
//!
//! ## Decode
//!
//! 1. LZ4 decompress → shuffled bytes (with self-describing prefix).
//! 2. `ByteShuffle::decode` reads the prefix and unshuffles → original data.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::codec::Codec;
use crate::codec::{CODEC_BLOSC2_SHUFFLE_LZ4, CODEC_LZ4};
use crate::error::CoreError;
use omnizip_filters::Filter;

/// Shuffle+LZ4 codec. Default item_size = 4 (float32).
pub struct ShuffleLz4Codec {
    item_size: usize,
}

impl ShuffleLz4Codec {
    #[must_use]
    pub fn new(item_size: usize) -> Self {
        let item_size = if [1, 2, 4, 8].contains(&item_size) {
            item_size
        } else {
            4 // default to f32
        };
        Self { item_size }
    }

    /// Default: float32 (item_size = 4). Covers most scientific data.
    #[must_use]
    pub fn float32() -> Self {
        Self::new(4)
    }

    /// float64 (item_size = 8). For double-precision scientific data.
    #[must_use]
    pub fn float64() -> Self {
        Self::new(8)
    }
}

impl Default for ShuffleLz4Codec {
    fn default() -> Self {
        Self::float32()
    }
}

impl Codec for ShuffleLz4Codec {
    fn id(&self) -> u8 {
        CODEC_BLOSC2_SHUFFLE_LZ4
    }

    fn name(&self) -> &'static str {
        "shuffle+lz4"
    }

    fn min_compress_size(&self) -> usize {
        512
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        let filter = omnizip_filters::shuffle::ByteShuffle::new(self.item_size);
        let shuffled = filter.encode(plaintext);
        let lz4_compressed = crate::codec::compress(crate::codec::CODEC_LZ4, &shuffled)?;
        // Prefix with the shuffled length so the decoder can tell LZ4
        // exactly how many bytes to produce (LZ4 validates output length).
        let shuffled_len = u32::try_from(shuffled.len()).unwrap_or(u32::MAX);
        let mut out = Vec::with_capacity(4 + lz4_compressed.len());
        out.extend_from_slice(&shuffled_len.to_le_bytes());
        out.extend_from_slice(&lz4_compressed);
        Ok(out)
    }

    fn decompress(&self, compressed: &[u8], _expected_len: u32) -> Result<Vec<u8>, CoreError> {
        if compressed.len() < 4 {
            return Err(CoreError::Corrupt {
                reason: "shuffle+lz4: truncated header".into(),
            });
        }
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&compressed[..4]);
        let shuffled_len = u32::from_le_bytes(len_bytes);
        let lz4_bytes = &compressed[4..];
        let shuffled = crate::codec::decompress(crate::codec::CODEC_LZ4, lz4_bytes, shuffled_len)?;
        let filter = omnizip_filters::shuffle::ByteShuffle::new(self.item_size);
        let recovered = filter.decode(&shuffled);
        Ok(recovered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_float32_array() {
        // 1024 float32 values with smooth gradient — the workload
        // shuffle is designed for.
        let samples: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.1).sin()).collect();
        let bytes: Vec<u8> = samples.iter().flat_map(|f| f.to_le_bytes()).collect();
        let codec = ShuffleLz4Codec::float32();
        let compressed = codec.compress(&bytes).expect("compress");
        let recovered = codec
            .decompress(&compressed, bytes.len() as u32)
            .expect("decompress");
        assert_eq!(recovered, bytes);
    }

    #[test]
    fn beats_plain_lz4_on_smooth_floats() {
        let samples: Vec<f32> = (0..4096).map(|i| (i as f32 * 0.01).sin()).collect();
        let bytes: Vec<u8> = samples.iter().flat_map(|f| f.to_le_bytes()).collect();
        let codec = ShuffleLz4Codec::float32();
        let shuffled_compressed = codec.compress(&bytes).expect("shuffle+lz4");
        let plain = crate::codec::compress(crate::codec::CODEC_LZ4, &bytes).expect("plain lz4");
        assert!(
            shuffled_compressed.len() < plain.len(),
            "shuffle+lz4 ({}) should beat plain LZ4 ({}) on smooth floats",
            shuffled_compressed.len(),
            plain.len()
        );
    }
}
