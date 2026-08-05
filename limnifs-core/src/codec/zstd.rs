//! Zstandard codec (0x02): pure Rust via `omnizip-zstd` 0.14.8.
//!
//! `omnizip-zstd` 0.14.8 ships a real encoder and decoder. The
//! `Default` (L6) and higher levels have an upstream regression on
//! some inputs (highly-repetitive text, small binary patterns) where
//! they produce 500× larger output and run 70,000× slower than
//! `Fast` (L3). See `docs/omnizip-proposals/zstd-default-broken.md`
//! for the bug report and acceptance criteria.
//!
//! As a temporary workaround we cap all quality levels at `Fast`
//! (L3). Decompression is unaffected — ZSTD's wire format is
//! level-independent. When the upstream fix lands, restore the
//! `Default`/`Better`/`Best` mapping in `level_for_quality`.

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
        // quality 3 → ZstdLevel::Fast (omnizip's L3).
        // We pin to L3 because omnizip-zstd 0.14.8's Default (L6) is
        // broken on some inputs. Restore to 6 when upstream fixes.
        Self { quality: 3 }
    }
}

/// Map a per-codec quality value to an `omnizip_zstd::ZstdLevel`.
///
/// **Workaround:** upper levels (`Default`, `Better`, `Best`) are
/// capped to `Fast` because omnizip-zstd 0.14.8 has a regression on
/// those levels for highly-repetitive inputs. `Fastest` and `Fast`
/// are unaffected. When the upstream fix lands, restore the
/// `6..=11 → Default`, `12..=21 → Better`, `22+ → Best` branches.
fn level_for_quality(quality: u8) -> omnizip_zstd::ZstdLevel {
    match quality {
        0..=2 => omnizip_zstd::ZstdLevel::Fastest,
        _ => omnizip_zstd::ZstdLevel::Fast,
    }
}

/// ZSTD codec. Encode at `Fast` (L3); decode at any level.
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
        let quality = if t.quality > 0 { t.quality } else { 3 };
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

/// Compress with Zstandard at `Fast` (level 3). Decompression is
/// level-independent so this only affects what we produce, not what
/// we can read.
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] only on internal encoder failure.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn compress(plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    omnizip_zstd::compress(plaintext, omnizip_zstd::ZstdLevel::Fast).map_err(|e| CoreError::Corrupt {
        reason: format!("zstd compress failed: {e}"),
    })
}

/// Compress at an explicit level. Used by callers that want a
/// different speed/ratio tradeoff than the default L3.
#[allow(dead_code)]
pub(crate) fn compress_at_level(
    plaintext: &[u8],
    level: omnizip_zstd::ZstdLevel,
) -> Result<Vec<u8>, CoreError> {
    omnizip_zstd::compress(plaintext, level).map_err(|e| CoreError::Corrupt {
        reason: format!("zstd compress (level {level}) failed: {e}"),
    })
}
