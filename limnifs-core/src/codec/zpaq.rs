//! ZPAQ codec (id 0x0B): context-mixing archiver.
//!
//! Wraps `omnizip-zpaq` 0.11. Phase 2: multi-model context mixing
//! (4 prediction models + adaptive mixer). Validated ratios: 0.57%
//! on repetitive text (4.5 MB), 47.86% on Rust source (511 KB).
//!
//! Clean-room implementation from Matt Mahoney's public-domain
//! ZPAQ specification. No GPL code in the source tree.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::codec::Codec;
use crate::codec::CODEC_ZPAQ;
use crate::error::CoreError;

pub struct ZpaqCodec;

impl Codec for ZpaqCodec {
    fn id(&self) -> u8 {
        CODEC_ZPAQ
    }
    fn name(&self) -> &'static str {
        "zpaq"
    }

    fn min_compress_size(&self) -> usize {
        4096
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        Ok(omnizip_zpaq::compress(plaintext))
    }

    fn decompress(&self, compressed: &[u8], _expected_len: u32) -> Result<Vec<u8>, CoreError> {
        omnizip_zpaq::decompress(compressed).map_err(|e| CoreError::Corrupt {
            reason: format!("zpaq decompress: {e}"),
        })
    }
}
