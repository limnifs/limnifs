//! Stub codec wrappers for reserved-but-not-yet-shipped codec ids.
//!
//! Each wrapper:
//! - Implements the `Codec` trait.
//! - Encode returns `UnsupportedFeature` with a clear "what's
//!   missing" message.
//! - Decode returns `UnsupportedFeature` too — these codecs are
//!   truly not implemented yet, so even if a slab somehow claimed
//!   to use them, we'd have to refuse.
//!
//! When the real codec lands in omnizip-rs, replace each stub with
//! a thin wrapper that delegates to the omnizip crate. The codec id
//! allocation does not change; the wire format is stable from day
//! one.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::codec::Codec;
use crate::error::CoreError;
use crate::codec::{CODEC_FLAC, CODEC_RICEPP};

/// Codec 0x07 — FLAC for PCM audio. Stub until `omnizip-flac` ships
/// a real LPC + Rice residual codec (current 0.4 has only the PCM
/// header parsers + raw PCM container).
pub struct FlacCodec;

impl Codec for FlacCodec {
    fn id(&self) -> u8 {
        CODEC_FLAC
    }
    fn name(&self) -> &'static str {
        "flac"
    }
    fn compress(&self, _plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        Err(CoreError::UnsupportedFeature {
            feature: "codec 0x07 (FLAC): omnizip-flac 0.4 ships only PCM header parsers; \
                      LPC + Rice codec still pending (see docs/omnizip-vs-limnifs-boundary.md)"
                .to_string(),
        })
    }
    fn decompress(&self, _compressed: &[u8], _expected_len: u32) -> Result<Vec<u8>, CoreError> {
        Err(CoreError::UnsupportedFeature {
            feature: "codec 0x07 (FLAC): omnizip-flac 0.4 ships only PCM header parsers"
                .to_string(),
        })
    }
}

/// Codec 0x08 — Rice++ for FITS images. **Wire-flipped to the real
/// omnizip-ricepp codec.** Kept as a fallback name for any future
/// reservation pattern; the real implementation lives in
/// `codec::ricepp::RiceppCodec`.
pub struct RiceppCodec;

impl Codec for RiceppCodec {
    fn id(&self) -> u8 {
        CODEC_RICEPP
    }
    fn name(&self) -> &'static str {
        "ricepp"
    }
    fn compress(&self, _plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        Err(CoreError::UnsupportedFeature {
            feature: "codec 0x08 (Rice++): legacy stub — real codec in codec::ricepp"
                .to_string(),
        })
    }
    fn decompress(&self, _compressed: &[u8], _expected_len: u32) -> Result<Vec<u8>, CoreError> {
        Err(CoreError::UnsupportedFeature {
            feature: "codec 0x08 (Rice++): legacy stub — real codec in codec::ricepp"
                .to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flac_stub_returns_unsupported() {
        let c = FlacCodec;
        assert!(c.compress(b"abc").is_err());
        assert!(c.decompress(b"abc", 3).is_err());
        assert_eq!(c.id(), CODEC_FLAC);
    }
}

