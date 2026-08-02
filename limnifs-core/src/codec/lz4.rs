//! LZ4 codec (0x01): block format via `lz4_flex` (pure Rust).

use crate::codec::Codec;
use crate::error::CoreError;

/// LZ4 codec. Compressed payloads carry a 4-byte LE original-size
/// prefix so `lz4_flex::decompress_size_prepended` can allocate the
/// right buffer upfront.
pub struct Lz4Codec;

impl Codec for Lz4Codec {
    fn id(&self) -> u8 {
        super::CODEC_LZ4
    }

    fn name(&self) -> &'static str {
        "lz4"
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        Ok(compress_lz4_with_size(plaintext))
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
            reason: format!("decompress: expected_len {expected_len} exceeds usize"),
        })?;
        let result =
            lz4_flex::decompress_size_prepended(compressed).map_err(|e| CoreError::Corrupt {
                reason: format!("lz4 decompress failed: {e}"),
            })?;
        if result.len() != expected_us {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "lz4 decompress: result length {} does not match plaintext_len {expected_us}",
                    result.len()
                ),
            });
        }
        Ok(result)
    }
}

/// Compress with LZ4, prepending the original size as a 4-byte LE
/// header (the format `lz4_flex::decompress_size_prepended` expects).
#[must_use]
pub fn compress_lz4_with_size(plaintext: &[u8]) -> Vec<u8> {
    lz4_flex::compress_prepend_size(plaintext)
}
