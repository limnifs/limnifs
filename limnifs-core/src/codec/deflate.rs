//! DEFLATE codec (0x05): RFC 1951 via `omnizip-deflate` (pure Rust).
//!
//! Encodes zlib-framed DEFLATE streams (RFC 1950 wrapper around RFC 1951
//! body) at levels 0–9. Universal compatibility: gzip, zlib, PNG, HTTP
//! content-encoding. Lower ratio than ZSTD/Brotli/LZMA but universally
//! interoperable.
//!
//! `omnizip-deflate` wraps `miniz_oxide` internally. We go through the
//! omnizip API rather than calling `miniz_oxide` directly so the codec
//! stack stays first-party (omnizip) end-to-end.

use crate::codec::Codec;
use crate::error::CoreError;

/// Default compression level for the codec trait's `compress()` path.
/// Level 6 is `miniz_oxide`'s default and `zlib`'s default.
pub(crate) const DEFAULT_LEVEL: u8 = 6;

/// DEFLATE codec. Zlib-framed RFC 1951 streams.
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
        let codec = omnizip_deflate::DeflateCodec::new();
        let result =
            omnizip_codecs::Codec::decompress(&codec, compressed, expected_len).map_err(deflate_err)?;
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

fn deflate_err(e: omnizip_codecs::OmnizipError) -> CoreError {
    CoreError::Corrupt {
        reason: format!("deflate: {e}"),
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
/// Returns [`CoreError::Corrupt`] if the encoder fails (rare; the
/// underlying `miniz_oxide` encoder is infallible in practice).
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn compress(plaintext: &[u8], level: u8) -> Result<Vec<u8>, CoreError> {
    let codec = omnizip_deflate::DeflateCodec::new();
    let omnizip_level = omnizip_codecs::CompressionLevel::new(level_to_omnizip(level));
    omnizip_codecs::Codec::compress(&codec, plaintext, omnizip_level).map_err(deflate_err)
}

/// Map LimniFS's 0–9 level to omnizip-deflate's compression level enum.
/// omnizip-deflate delegates to miniz_oxide, which accepts 1–9 (with 0
/// treated as no-compression stored blocks in some versions; we clamp
/// to 1 to be safe).
fn level_to_omnizip(level: u8) -> u8 {
    match level {
        0 => 1,
        1..=9 => level,
        _ => 6,
    }
}
