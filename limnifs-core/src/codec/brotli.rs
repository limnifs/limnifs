//! Brotli codec (0x04): frame format via `omnizip-brotli`.
//!
//! Routes through the omnizip API end-to-end. As of omnizip 0.14.40 the
//! encoder is in-house (Phase C partial): quality 0–1 uses the fast
//! vendored path; quality 2–11 uses `compress_fragment`. Intermediate
//! quality levels do not yet differentiate ratio the way the C
//! reference does — see `docs/omnizip-proposals/brotli-phase-c.md`.
//!
//! The codec defaults to **quality 5** (fast path for the per-chunk
//! writer pipeline).

use crate::codec::{Codec, CodecTunables, PerCodecTunables};
use crate::error::CoreError;

/// Brotli quality 5 — fast mode, the right default for the per-chunk
/// writer pipeline. Quality 11 is available via [`compress`] for
/// archival use; the codec registry's default encoder uses this
/// constant.
pub(crate) const DEFAULT_QUALITY: i32 = 5;

/// Strongly-typed Brotli tunables.
#[derive(Clone, Debug)]
pub struct BrotliTunables {
    /// Quality 0..=11 (higher = better ratio, slower).
    pub quality: i32,
}

impl Default for BrotliTunables {
    fn default() -> Self {
        Self {
            quality: DEFAULT_QUALITY,
        }
    }
}

/// Brotli codec. Encode at quality 5 by default; decode at any quality.
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
        let codec = omnizip_brotli::BrotliCodec;
        let result =
            omnizip_codecs::Codec::decompress(&codec, compressed, expected_len).map_err(brotli_err)?;
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
        // quality is 0..=11 inclusive. Do NOT treat 0 as "unset" —
        // 0 is a valid Brotli quality (fastest/store-ish). Callers
        // that want the default leave CodecTunables::quality at the
        // Brotli default (5) via from_quality / Default.
        let q = i32::from(t.quality).clamp(0, 11);
        compress(plaintext, q)
    }
}

impl PerCodecTunables for BrotliCodec {
    type Tunables = BrotliTunables;

    fn compress_with_owned_tunables(
        &self,
        plaintext: &[u8],
        t: &Self::Tunables,
    ) -> Result<Vec<u8>, CoreError> {
        compress(plaintext, t.quality.clamp(0, 11))
    }
}

fn brotli_err(e: omnizip_codecs::OmnizipError) -> CoreError {
    CoreError::Corrupt {
        reason: format!("brotli: {e}"),
    }
}

/// Compress `plaintext` with Brotli at the given quality (0–11).
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if the Brotli encoder fails.
pub(crate) fn compress(plaintext: &[u8], quality: i32) -> Result<Vec<u8>, CoreError> {
    let codec = omnizip_brotli::BrotliCodec;
    let q = quality.clamp(0, 11) as u8;
    let level = omnizip_codecs::CompressionLevel::new(q);
    omnizip_codecs::Codec::compress(&codec, plaintext, level).map_err(brotli_err)
}

/// Decompress a Brotli stream. If `expected_len` is `u32::MAX`, the
/// underlying omnizip-brotli path performs its own length check.
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if decompression fails or the
/// result length does not match `expected_len`.
pub(crate) fn decompress_at_quality(
    compressed: &[u8],
    expected_len: u32,
) -> Result<Vec<u8>, CoreError> {
    let codec = omnizip_brotli::BrotliCodec;
    omnizip_codecs::Codec::decompress(&codec, compressed, expected_len).map_err(brotli_err)
}
