//! XZ/LZMA2 codec (0x03): decode-only in pure Rust via `omnizip-lzma`.
//!
//! The omnizip-lzma crate is a Rust port of omnizip's Ruby LZMA reference
//! (itself derived from tukaani-project/xz liblzma). Decode handles raw
//! LZMA2 chunk data as stored in `LimniFS` drop records. Encode returns
//! `UnsupportedFeature` until the LZMA encoder port is complete.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::codec::Codec;
use crate::error::CoreError;

/// XZ/LZMA2 codec. Decode-only.
pub struct XzCodec;

impl Codec for XzCodec {
    fn id(&self) -> u8 {
        super::CODEC_XZ
    }

    fn name(&self) -> &'static str {
        "xz"
    }

    fn compress(&self, _plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        Err(CoreError::UnsupportedFeature {
            feature: "compress codec 0x03 (xz): pure-Rust LZMA encoder does not exist; \
                      omnizip-lzma encoder port is in progress"
                .to_string(),
        })
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
            reason: format!("decompress: expected_len {expected_len} exceeds usize"),
        })?;
        let (result, _consumed) =
            omnizip_lzma::lzma2::decode_lzma2_stream(compressed).map_err(|e| {
                CoreError::Corrupt {
                    reason: format!("lzma2 decompress failed: {e}"),
                }
            })?;
        if result.len() != expected_us {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "lzma2 decompress: result length {} does not match plaintext_len {expected_us}",
                    result.len()
                ),
            });
        }
        Ok(result)
    }
}
