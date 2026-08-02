//! `PPMd` codec (id 0x0C): Prediction by Partial Matching.
//!
//! Wraps `omnizip-ppmd` 0.11. Uses an adaptive context model with
//! the ZPAQ arithmetic coder. Validated ratios: 2.28% on repetitive
//! text (4.5 MB), 45.86% on Rust source (511 KB).
//!
//! Clean-room implementation from Shkarin's DCC 2001 paper.
//! No LGPL code in the source tree.

use crate::codec::Codec;
use crate::codec::CODEC_PPMD;
use crate::error::CoreError;

pub struct PpmdCodec;

impl Codec for PpmdCodec {
    fn id(&self) -> u8 {
        CODEC_PPMD
    }
    fn name(&self) -> &'static str {
        "ppmd"
    }

    fn min_compress_size(&self) -> usize {
        4096
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        omnizip_ppmd::compress_default(plaintext).map_err(|e| CoreError::Corrupt {
            reason: format!("ppmd compress: {e}"),
        })
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let expected = usize::try_from(expected_len).unwrap_or(usize::MAX);
        omnizip_ppmd::decompress(compressed, expected).map_err(|e| CoreError::Corrupt {
            reason: format!("ppmd decompress: {e}"),
        })
    }
}
