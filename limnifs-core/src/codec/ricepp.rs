//! Rice++ codec (id 0x08): integer-pixel image compression.
//!
//! Wraps `omnizip-ricepp` 0.4. Rice++ applies Rice coding to the
//! byte-wise delta of consecutive pixel values. Optimal for
//! astronomical FITS images, sensor data, and any uncompressed
//! integer-pixel image with smooth local gradients.
//!
//! ## Codec parameters
//!
//! Because ricepp needs to know the pixel bit-depth + byte order
//! before it can encode, and our `Codec` trait takes only plaintext
//! bytes, the wrapper carries a `CodecConfig` baked in at construction
//! time. The categorizer passes the right config when registering
//! drops; the registry stores one ricepp codec per (pixel_bits,
//! byte_order) combination encountered.
//!
//! For round-trip through the generic registry (no config available),
//! the default codec uses 16-bit big-endian — matches the FITS
//! default and works for any input whose byte count is even.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::codec::Codec;
use crate::error::CoreError;
use crate::codec::CODEC_RICEPP;

const RICEPP_CODEC_ID: u16 = 0x08;

/// Rice++ codec with default 16-bit big-endian pixels (FITS default).
pub struct RiceppCodec {
    config: omnizip_ricepp::CodecConfig,
}

impl RiceppCodec {
    #[must_use]
    pub fn new(config: omnizip_ricepp::CodecConfig) -> Self {
        Self { config }
    }

    /// Default config: 16-bit big-endian pixels, block_size = 128.
    /// Matches the most common FITS image type (BITPIX = 16).
    #[must_use]
    pub fn fits_default() -> Self {
        Self::new(omnizip_ricepp::CodecConfig::default())
    }
}

impl Default for RiceppCodec {
    fn default() -> Self {
        Self::fits_default()
    }
}

impl Codec for RiceppCodec {
    fn id(&self) -> u8 {
        CODEC_RICEPP
    }
    fn name(&self) -> &'static str {
        "ricepp"
    }

    fn min_compress_size(&self) -> usize {
        1024
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        omnizip_ricepp::compress(plaintext, self.config).map_err(ricepp_err)
    }

    fn decompress(&self, compressed: &[u8], _expected_len: u32) -> Result<Vec<u8>, CoreError> {
        omnizip_ricepp::decompress(compressed).map_err(ricepp_err)
    }
}

fn ricepp_err(e: omnizip_codecs::OmnizipError) -> CoreError {
    let _ = RICEPP_CODEC_ID;
    CoreError::Corrupt {
        reason: format!("ricepp: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_synthetic_pixel_data() {
        // 256 pixels of smoothly-varying 16-bit values — exactly the
        // kind of data Rice++ is designed for.
        let pixels: Vec<u16> = (0..256).map(|i| (i * 17) & 0xFFFF).collect();
        let mut bytes = Vec::with_capacity(pixels.len() * 2);
        for p in &pixels {
            bytes.extend_from_slice(&p.to_be_bytes());
        }
        let codec = RiceppCodec::fits_default();
        let compressed = codec.compress(&bytes).expect("compress");
        let recovered = codec.decompress(&compressed, bytes.len() as u32).expect("decompress");
        assert_eq!(recovered, bytes);
    }

    #[test]
    fn rejects_input_with_wrong_pixel_width() {
        // Default config expects 16-bit pixels (2 bytes each).
        // 11 bytes is not a multiple of 2.
        let codec = RiceppCodec::fits_default();
        let result = codec.compress(&[0u8; 11]);
        assert!(result.is_err());
    }
}
