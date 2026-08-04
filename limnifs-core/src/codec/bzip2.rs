//! `BZip2` codec (id 0x10): Burrows-Wheeler Transform + Huffman.
//!
//! Wraps `omnizip-bzip2` 0.11. The omnizip codec handles BWT, MTF,
//! RLE, and Huffman coding internally. Default level is 9 (900 KB
//! block size).

use omnizip_codecs::{Codec as OmnizipCodec, CompressionLevel};

use crate::codec::{Codec, CodecTunables, PerCodecTunables, CODEC_BZIP2};
use crate::error::CoreError;

/// Default `BZip2` compression level (1..=9, higher = better ratio).
const DEFAULT_BZIP2_LEVEL: u8 = 9;

/// Strongly-typed BZip2 tunables.
#[derive(Clone, Debug)]
pub struct Bzip2Tunables {
    /// Block size in KB (100..=900). Maps to level 1..=9.
    pub block_kb: u32,
}

impl Default for Bzip2Tunables {
    fn default() -> Self {
        Self {
            block_kb: u32::from(DEFAULT_BZIP2_LEVEL) * 100,
        }
    }
}

pub struct Bzip2Codec {
    level: CompressionLevel,
}

impl Bzip2Codec {
    #[must_use]
    pub fn new() -> Self {
        Self {
            level: CompressionLevel::from(DEFAULT_BZIP2_LEVEL),
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn with_level(level: u8) -> Self {
        Self {
            level: CompressionLevel::from(level.clamp(1, 9)),
        }
    }
}

impl Default for Bzip2Codec {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec for Bzip2Codec {
    fn id(&self) -> u8 {
        CODEC_BZIP2
    }

    fn name(&self) -> &'static str {
        "bzip2"
    }

    fn min_compress_size(&self) -> usize {
        1024
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        let codec = omnizip_bzip2::Bzip2Codec::new();
        OmnizipCodec::compress(&codec, plaintext, self.level).map_err(bzip2_err)
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let codec = omnizip_bzip2::Bzip2Codec::new();
        OmnizipCodec::decompress(&codec, compressed, expected_len).map_err(bzip2_err)
    }

    fn compress_with_tunables(
        &self,
        plaintext: &[u8],
        t: &CodecTunables,
    ) -> Result<Vec<u8>, CoreError> {
        // Bzip2 block size in 100 KB increments (1..=9). Profiles
        // declare `block_size_kb` (e.g. 900 for max-ratio); map to
        // the closest valid level.
        let level = if t.bzip2_block_kb > 0 {
            let kb = t.bzip2_block_kb.clamp(100, 900);
            CompressionLevel::from(((kb + 99) / 100) as u8)
        } else {
            self.level
        };
        let codec = omnizip_bzip2::Bzip2Codec::new();
        OmnizipCodec::compress(&codec, plaintext, level).map_err(bzip2_err)
    }
}

impl PerCodecTunables for Bzip2Codec {
    type Tunables = Bzip2Tunables;

    fn compress_with_owned_tunables(
        &self,
        plaintext: &[u8],
        t: &Self::Tunables,
    ) -> Result<Vec<u8>, CoreError> {
        let kb = t.block_kb.clamp(100, 900);
        let level = CompressionLevel::from(((kb + 99) / 100) as u8);
        let codec = omnizip_bzip2::Bzip2Codec::new();
        OmnizipCodec::compress(&codec, plaintext, level).map_err(bzip2_err)
    }
}

fn bzip2_err(e: omnizip_codecs::OmnizipError) -> CoreError {
    CoreError::Corrupt {
        reason: format!("bzip2: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_text() {
        let input = b"the quick brown fox jumps over the lazy dog. ".repeat(1000);
        let codec = Bzip2Codec::new();
        let compressed = codec.compress(&input).expect("compress");
        let decompressed = codec
            .decompress(&compressed, input.len() as u32)
            .expect("decompress");
        assert_eq!(decompressed, input);
        assert!(compressed.len() < input.len());
    }

    #[test]
    fn round_trip_empty() {
        let codec = Bzip2Codec::new();
        let compressed = codec.compress(&[]).expect("compress");
        let decompressed = codec.decompress(&compressed, 0).expect("decompress");
        assert!(decompressed.is_empty());
    }

    #[test]
    fn rejects_bad_input() {
        let codec = Bzip2Codec::new();
        assert!(codec.decompress(b"not bzip2 data", 100).is_err());
    }
}
