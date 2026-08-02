//! Zstandard codec (0x02): pure Rust via `omnizip-zstd` 0.5.
//!
//! `omnizip-zstd` 0.5 ships a real Phase C encoder (match finder +
//! FSE sequences + Huffman literals) and a complete decoder. This
//! wrapper defaults to ZSTD `Default` (level 6 — libzstd's default)
//! for the right speed/ratio balance on source code.
//!
//! ## Why not ruzstd any more
//!
//! ruzstd's encoder only implements level 1 and its decoder had
//! historical bugs with compressed literals. omnizip-zstd 0.5 has
//! both paths complete and differentially tested against the C
//! reference; switching gives us levels 1, 3, 6, 12, 22 with no
//! quality regression.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::codec::Codec;
use crate::error::CoreError;

/// ZSTD codec. Encode at `Default` (L6); decode at any level.
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
        let result = omnizip_zstd::decompress(compressed, expected_len).map_err(|e| {
            CoreError::Corrupt {
                reason: format!("zstd decompress failed: {e}"),
            }
        })?;
        let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
            reason: format!("decompress: expected_len {expected_len} exceeds usize"),
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

/// Compress with Zstandard at `Default` (level 6). This matches
/// libzstd's CLI default and gives the right speed/ratio balance
/// for source code, CSV, and general text.
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] only on internal encoder failure.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn compress(plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    omnizip_zstd::compress(plaintext, omnizip_zstd::ZstdLevel::Default).map_err(|e| {
        CoreError::Corrupt {
            reason: format!("zstd compress failed: {e}"),
        }
    })
}

/// Compress at an explicit level. Used by callers that want a
/// different speed/ratio tradeoff than the default L6.
#[allow(dead_code)]
pub(crate) fn compress_at_level(
    plaintext: &[u8],
    level: omnizip_zstd::ZstdLevel,
) -> Result<Vec<u8>, CoreError> {
    omnizip_zstd::compress(plaintext, level).map_err(|e| CoreError::Corrupt {
        reason: format!("zstd compress (level {level}) failed: {e}"),
    })
}
