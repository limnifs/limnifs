//! Zstandard codec (0x02): pure Rust via `omnizip-zstd` 0.14.10.
//!
//! `omnizip-zstd` 0.14.10 ships a real encoder and decoder. The
//! `Default` (L6), `Better` (L12), and `Best` (L22) levels had a
//! regression in 0.14.8 on highly-repetitive inputs (50 KB output
//! and 14+ s runtime for 90 KB of repeated text). The 0.14.10 fix
//! (omnizip-rs PR #90) restores correct level differentiation; see
//! `docs/omnizip-proposals/zstd-default-broken.md` for the original
//! report.

use crate::codec::{Codec, CodecTunables, PerCodecTunables};
use crate::error::CoreError;

/// Strongly-typed ZSTD tunables.
#[derive(Clone, Debug)]
pub struct ZstdTunables {
    /// ZSTD compression level (mapped to omnizip_zstd::ZstdLevel).
    pub quality: u8,
}

impl Default for ZstdTunables {
    fn default() -> Self {
        // quality 6 → ZstdLevel::Default (libzstd's default).
        Self { quality: 6 }
    }
}

/// Map a per-codec quality value to an `omnizip_zstd::ZstdLevel`.
fn level_for_quality(quality: u8) -> omnizip_zstd::ZstdLevel {
    match quality {
        0..=2 => omnizip_zstd::ZstdLevel::Fastest,
        3..=5 => omnizip_zstd::ZstdLevel::Fast,
        6..=11 => omnizip_zstd::ZstdLevel::Default,
        12..=21 => omnizip_zstd::ZstdLevel::Better,
        _ => omnizip_zstd::ZstdLevel::Best,
    }
}

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
        let result =
            omnizip_zstd::decompress(compressed, expected_len).map_err(|e| CoreError::Corrupt {
                reason: format!("zstd decompress failed: {e}"),
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

    fn compress_with_tunables(
        &self,
        plaintext: &[u8],
        t: &CodecTunables,
    ) -> Result<Vec<u8>, CoreError> {
        let quality = if t.quality > 0 { t.quality } else { 6 };
        let level = level_for_quality(quality);
        omnizip_zstd::compress(plaintext, level).map_err(|e| CoreError::Corrupt {
            reason: format!("zstd compress (level {level}) failed: {e}"),
        })
    }
}

impl PerCodecTunables for ZstdCodec {
    type Tunables = ZstdTunables;

    fn compress_with_owned_tunables(
        &self,
        plaintext: &[u8],
        t: &Self::Tunables,
    ) -> Result<Vec<u8>, CoreError> {
        let level = level_for_quality(t.quality);
        omnizip_zstd::compress(plaintext, level).map_err(|e| CoreError::Corrupt {
            reason: format!("zstd compress (level {level}) failed: {e}"),
        })
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
