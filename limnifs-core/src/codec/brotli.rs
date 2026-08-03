//! Brotli codec (0x04): frame format via the `brotli` crate (pure Rust,
//! by Daniel Reiter Horn — the format's original author).
//!
//! The codec defaults to **quality 5**, Brotli's standard fast mode.
//! This is the right tradeoff for `LimniFS`'s per-chunk pipeline: fast
//! enough to keep create throughput competitive with `SquashFS`'s zstd
//! L1, while beating ZSTD L1 (ruzstd-bounded) on text/source ratio.
//! The future `--codec-map` flag (roadmap item 06) will allow callers
//! to opt into q11 for archival workloads where create speed doesn't
//! matter.

use std::io::Cursor;

use crate::codec::{Codec, CodecTunables};
use crate::error::CoreError;

/// Brotli quality 5 — fast mode, the right default for the per-chunk
/// writer pipeline. Quality 11 is available via [`compress`] for
/// archival use; the codec registry's default encoder uses this
/// constant.
pub(crate) const DEFAULT_QUALITY: i32 = 5;

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

    fn compress_with_tunables(
        &self,
        plaintext: &[u8],
        t: &CodecTunables,
    ) -> Result<Vec<u8>, CoreError> {
        let q = if t.quality > 0 {
            i32::from(t.quality).clamp(0, 11)
        } else {
            DEFAULT_QUALITY
        };
        compress(plaintext, q)
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

/// Decompress a Brotli stream. If `expected_len` is `u32::MAX`, skip
/// the length check (used by composite codecs that don't know the
/// intermediate length ahead of time).
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if decompression fails or the
/// result length does not match `expected_len` (when checked).
pub(crate) fn decompress_at_quality(
    compressed: &[u8],
    expected_len: u32,
) -> Result<Vec<u8>, CoreError> {
    let mut result = Vec::new();
    brotli::BrotliDecompress(&mut Cursor::new(compressed), &mut result).map_err(|e| {
        CoreError::Corrupt {
            reason: format!("brotli decompress failed: {e}"),
        }
    })?;
    if expected_len != u32::MAX {
        let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
            reason: format!("brotli: expected_len {expected_len} exceeds usize"),
        })?;
        if result.len() != expected_us {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "brotli decompress: result length {} does not match plaintext_len {expected_us}",
                    result.len()
                ),
            });
        }
    }
    Ok(result)
}
