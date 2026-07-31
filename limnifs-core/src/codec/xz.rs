//! XZ/LZMA2 codec (0x03): decode-only in pure Rust via `lzma-rs`.
//!
//! `lzma-rs` 0.3.0's encoders are non-compressing stubs (see the parent
//! module docs). Encode returns `UnsupportedFeature` so callers route to
//! ZSTD for fresh compression; decode reads legacy XZ-encoded drops
//! produced by external tooling.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::codec::Codec;
use crate::error::CoreError;

/// XZ/LZMA2 codec. Decode-only.
pub struct XzCodec;

impl Codec for XzCodec {
    fn id(&self) -> u8 {
        super::CODEC_XZ
    }

    fn name(&self) -> &'static str {
        "xz"
    }

    fn compress(&self, _plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        Err(CoreError::UnsupportedFeature {
            feature: "compress codec 0x03 (xz): pure-Rust LZMA encoder does not exist; \
                      lzma-rs 0.3.0's encoder is a non-compressing stub"
                .to_string(),
        })
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
            reason: format!("decompress: expected_len {expected_len} exceeds usize"),
        })?;
        let mut result = Vec::with_capacity(expected_us);
        lzma_rs::lzma2_decompress(&mut std::io::Cursor::new(compressed), &mut result).map_err(
            |e| CoreError::Corrupt {
                reason: format!("lzma2 decompress failed: {e}"),
            },
        )?;
        if result.len() != expected_us {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "lzma2 decompress: result length {} does not match plaintext_len {expected_us}",
                    result.len()
                ),
            });
        }
        Ok(result)
    }
}
