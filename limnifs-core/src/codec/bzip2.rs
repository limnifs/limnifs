//! BZip2 codec (id 0x10): Burrows-Wheeler Transform + Huffman.
//!
//! Wraps `omnizip-bzip2` 0.11. The omnizip codec handles BWT, MTF,
//! RLE, and Huffman coding internally. Default level is 9 (900 KB
//! block size).

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use omnizip_codecs::{Codec as OmnizipCodec, CompressionLevel};

use crate::codec::Codec;
use crate::codec::CODEC_BZIP2;
use crate::error::CoreError;

/// Default BZip2 compression level (1..=9, higher = better ratio).
const DEFAULT_BZIP2_LEVEL: u8 = 9;

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
