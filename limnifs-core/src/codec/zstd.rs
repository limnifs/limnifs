//! Zstandard codec (0x02): frame format via `ruzstd` (pure Rust).
//!
//! Encode uses `CompressionLevel::Fastest` (ZSTD level 1) — ruzstd
//! 0.9.0's only implemented encode level. Decode supports any level
//! the reference ZSTD encoder can produce.
//!
//! omnizip-zstd (from omnizip/omnizip-rs) handles Raw, RLE, and simple
//! Compressed blocks but does not yet support Huffman-coded literals.
//! It will replace ruzstd once the Huffman path is ported. The omnizip-zstd
//! differential parity tests pass on all golden fixtures from facebook/zstd.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::codec::Codec;
use crate::error::CoreError;

/// ZSTD codec. Encode at `Fastest`; decode at any level.
pub struct ZstdCodec;

impl Codec for ZstdCodec {
    fn id(&self) -> u8 {
        super::CODEC_ZSTD
    }

    fn name(&self) -> &'static str {
        "zstd"
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        compress(plaintext)
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        use std::io::Read as _;
        let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
            reason: format!("decompress: expected_len {expected_len} exceeds usize"),
        })?;
        let decoder = ruzstd::decoding::StreamingDecoder::new(compressed).map_err(|e| {
            CoreError::Corrupt {
                reason: format!("zstd decompress (init) failed: {e}"),
            }
        })?;
        let mut result = Vec::with_capacity(expected_us);
        decoder
            .take(u64::from(expected_len))
            .read_to_end(&mut result)
            .map_err(|e| CoreError::Corrupt {
                reason: format!("zstd decompress failed: {e}"),
            })?;
        if result.len() != expected_us {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "zstd decompress: result length {} does not match plaintext_len {expected_us}",
                    result.len()
                ),
            });
        }
        Ok(result)
    }
}

/// Compress with Zstandard at `CompressionLevel::Fastest` (ZSTD level 1).
/// The output is a standard ZSTD frame decodable by any conformant ZSTD
/// decoder.
///
/// `ruzstd::encoding::compress_to_vec` is infallible, so this wrapper
/// never fails; the `Result` is kept for symmetry with the other codec
/// helpers.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn compress(plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    Ok(ruzstd::encoding::compress_to_vec(
        plaintext,
        ruzstd::encoding::CompressionLevel::Fastest,
    ))
}
