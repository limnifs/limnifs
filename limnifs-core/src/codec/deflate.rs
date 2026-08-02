//! DEFLATE codec (0x05): RFC 1951 via `miniz_oxide` (pure Rust).
//!
//! Encodes raw DEFLATE streams (no zlib/gzip container) at levels 0–9.
//! Universal compatibility: gzip, zlib, PNG, HTTP content-encoding.
//! Lower ratio than ZSTD/Brotli/LZMA but universally interoperable.

use crate::codec::Codec;
use crate::error::CoreError;

/// Default compression level for the codec trait's `compress()` path.
/// Level 6 is `miniz_oxide`'s default and `zlib`'s default.
pub(crate) const DEFAULT_LEVEL: u8 = 6;

/// DEFLATE codec. Raw RFC 1951 streams, no container.
pub struct DeflateCodec;

impl Codec for DeflateCodec {
    fn id(&self) -> u8 {
        super::CODEC_DEFLATE
    }

    fn name(&self) -> &'static str {
        "deflate"
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        compress(plaintext, DEFAULT_LEVEL)
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
            reason: format!("decompress: expected_len {expected_len} exceeds usize"),
        })?;
        let result = miniz_oxide::inflate::decompress_to_vec_zlib(compressed).map_err(|e| {
            CoreError::Corrupt {
                reason: format!("deflate decompress failed: {e:?}"),
            }
        })?;
        if result.len() != expected_us {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "deflate decompress: result length {} does not match plaintext_len {expected_us}",
                    result.len()
                ),
            });
        }
        Ok(result)
    }
}

/// Compress `plaintext` with DEFLATE at the given level (0–9).
///
/// The output is a zlib-framed DEFLATE stream (RFC 1950 2-byte header +
/// RFC 1951 DEFLATE body + 4-byte Adler-32 checksum) so it round-trips
/// through any zlib decoder.
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if the encoder fails (rare; `miniz_oxide`
/// is infallible in practice).
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn compress(plaintext: &[u8], level: u8) -> Result<Vec<u8>, CoreError> {
    let miniz_level = level_to_miniz(level);
    Ok(miniz_oxide::deflate::compress_to_vec_zlib(
        plaintext,
        miniz_level,
    ))
}

/// Map `LimniFS`'s 0–9 level to `miniz_oxide`'s compression level enum.
fn level_to_miniz(level: u8) -> u8 {
    match level {
        0 => 1,
        1..=9 => level,
        _ => 6,
    }
}
