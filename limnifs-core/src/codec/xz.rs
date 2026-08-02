//! XZ/LZMA2 codec (0x03): pure Rust via `omnizip-lzma` 0.5.
//!
//! `omnizip-lzma` 0.5 ships a Phase C encoder: the match finder is
//! wired into `Lzma1Encoder::encode` with greedy parsing + rep0
//! tracking + matched-literal context. Real compression works;
//! output is smaller than the input on typical source-code/text
//! payloads. Round-trips through `xz_decompress`.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::codec::Codec;
use crate::error::CoreError;

/// XZ/LZMA2 codec. Encode and decode both via `omnizip-lzma` 0.5
/// (Phase C — match finder + greedy parser).
pub struct XzCodec;

impl Codec for XzCodec {
    fn id(&self) -> u8 {
        super::CODEC_XZ
    }

    fn name(&self) -> &'static str {
        "xz"
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        omnizip_lzma::xz_compress(plaintext).map_err(|e| CoreError::Corrupt {
            reason: format!("xz compress failed: {e}"),
        })
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
            reason: format!("decompress: expected_len {expected_len} exceeds usize"),
        })?;
        let result = omnizip_lzma::xz_container::xz_decompress(compressed).map_err(|e| {
            CoreError::Corrupt {
                reason: format!("xz decompress failed: {e}"),
            }
        })?;
        if result.len() != expected_us {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "xz decompress: result length {} does not match plaintext_len {expected_us}",
                    result.len()
                ),
            });
        }
        Ok(result)
    }
}
