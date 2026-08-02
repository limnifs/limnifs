//! Deflate64 codec (id 0x11): extended DEFLATE with 64 KB window.
//!
//! Wraps `omnizip-deflate64` 0.11. Used for ZIP method 9 entries
//! (Deflate64) where the standard Deflate window (32 KB) is too
//! small for larger files.

use omnizip_codecs::{Codec as OmnizipCodec, CompressionLevel};

use crate::codec::Codec;
use crate::codec::CODEC_DEFLATE64;
use crate::error::CoreError;

/// Default Deflate64 compression level (1..=9).
const DEFAULT_DEFLATE64_LEVEL: u8 = 6;

pub struct Deflate64Codec {
    level: CompressionLevel,
}

impl Deflate64Codec {
    #[must_use]
    pub fn new() -> Self {
        Self {
            level: CompressionLevel::from(DEFAULT_DEFLATE64_LEVEL),
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

impl Default for Deflate64Codec {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec for Deflate64Codec {
    fn id(&self) -> u8 {
        CODEC_DEFLATE64
    }

    fn name(&self) -> &'static str {
        "deflate64"
    }

    fn min_compress_size(&self) -> usize {
        1024
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        let codec = omnizip_deflate64::Deflate64Codec;
        OmnizipCodec::compress(&codec, plaintext, self.level).map_err(deflate64_err)
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let codec = omnizip_deflate64::Deflate64Codec;
        OmnizipCodec::decompress(&codec, compressed, expected_len).map_err(deflate64_err)
    }
}

fn deflate64_err(e: omnizip_codecs::OmnizipError) -> CoreError {
    CoreError::Corrupt {
        reason: format!("deflate64: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_text() {
        let input = b"the quick brown fox jumps over the lazy dog. ".repeat(1000);
        let codec = Deflate64Codec::new();
        let compressed = codec.compress(&input).expect("compress");
        let decompressed = codec
            .decompress(&compressed, input.len() as u32)
            .expect("decompress");
        assert_eq!(decompressed, input);
        assert!(compressed.len() < input.len());
    }

    #[test]
    fn round_trip_empty() {
        let codec = Deflate64Codec::new();
        let compressed = codec.compress(&[]).expect("compress");
        let decompressed = codec.decompress(&compressed, 0).expect("decompress");
        assert!(decompressed.is_empty());
    }
}
