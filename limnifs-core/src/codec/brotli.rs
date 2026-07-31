//! Brotli codec (0x04): frame format via the `brotli` crate (pure Rust,
//! by Daniel Reiter Horn — the format's original author).
//!
//! The codec uses a fixed quality of **11** (Brotli's maximum), making
//! this the highest-ratio pure-Rust codec in the registry. Encode time
//! at quality 11 is seconds-per-MB; users who want speed should select
//! ZSTD (0x02) or LZ4 (0x01). The future `--codec-map` flag (roadmap
//! item 06) will allow per-class routing to different Brotli qualities.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::io::Cursor;

use crate::codec::Codec;
use crate::error::CoreError;

/// Brotli quality 11 (the reference encoder's maximum). Produces the
/// smallest output at the cost of slow encoding.
pub(crate) const DEFAULT_QUALITY: i32 = 11;

/// Brotli codec. Encode at quality 11; decode at any quality.
pub struct BrotliCodec;

impl Codec for BrotliCodec {
    fn id(&self) -> u8 {
        super::CODEC_BROTLI
    }

    fn name(&self) -> &'static str {
        "brotli"
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        compress(plaintext, DEFAULT_QUALITY)
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
            reason: format!("decompress: expected_len {expected_len} exceeds usize"),
        })?;
        let mut result = Vec::with_capacity(expected_us);
        brotli::BrotliDecompress(&mut Cursor::new(compressed), &mut result).map_err(|e| {
            CoreError::Corrupt {
                reason: format!("brotli decompress failed: {e}"),
            }
        })?;
        if result.len() != expected_us {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "brotli decompress: result length {} does not match plaintext_len {expected_us}",
                    result.len()
                ),
            });
        }
        Ok(result)
    }
}

/// Compress `plaintext` with Brotli at the given quality (0–11).
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if the Brotli encoder fails.
pub(crate) fn compress(plaintext: &[u8], quality: i32) -> Result<Vec<u8>, CoreError> {
    let params = brotli::enc::backward_references::BrotliEncoderParams {
        quality,
        ..Default::default()
    };
    let mut result = Vec::new();
    brotli::BrotliCompress(&mut Cursor::new(plaintext), &mut result, &params).map_err(|e| {
        CoreError::Corrupt {
            reason: format!("brotli compress (quality {quality}) failed: {e}"),
        }
    })?;
    Ok(result)
}
