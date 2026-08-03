//! PPMd8 codec (id 0x12): PPM with RESTART restoration + RLE.
//!
//! Wraps `omnizip-ppmd` 0.13 `ppmd8` module. User-tunable memory
//! budget (default 64 MB) and context order (default 6).
//!
//! PPMd8 improves over PPMd7 with:
//! - RESTART restoration method (faster recovery from context pollution)
//! - Built-in RLE for runs of identical bytes
//! - Trie pruning for bounded memory usage

use crate::codec::{Codec, CODEC_PPMD8};
use crate::error::CoreError;

/// Default PPMd8 memory budget: 64 MB.
pub const DEFAULT_PPMD8_BUDGET: usize = 64 * 1024 * 1024;
/// Default PPMd8 context order.
pub const DEFAULT_PPMD8_ORDER: u8 = 6;

pub struct Ppmd8Codec {
    order: u8,
    budget: usize,
}

impl Ppmd8Codec {
    #[must_use]
    pub fn new() -> Self {
        Self {
            order: DEFAULT_PPMD8_ORDER,
            budget: DEFAULT_PPMD8_BUDGET,
        }
    }

    /// Set a custom memory budget. Larger budget = better ratio.
    #[must_use]
    pub fn with_budget(mut self, budget: usize) -> Self {
        self.budget = budget;
        self
    }

    /// Set the context model order (2..=16).
    #[must_use]
    pub fn with_order(mut self, order: u8) -> Self {
        self.order = order;
        self
    }
}

impl Default for Ppmd8Codec {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec for Ppmd8Codec {
    fn id(&self) -> u8 {
        CODEC_PPMD8
    }
    fn name(&self) -> &'static str {
        "ppmd8"
    }

    fn min_compress_size(&self) -> usize {
        4096
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        omnizip_ppmd::ppmd8::compress_with_budget(plaintext, self.order, self.budget).map_err(|e| {
            CoreError::Corrupt {
                reason: format!("ppmd8 compress: {e}"),
            }
        })
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let expected = usize::try_from(expected_len).unwrap_or(usize::MAX);
        omnizip_ppmd::ppmd8::decompress(compressed, expected).map_err(|e| CoreError::Corrupt {
            reason: format!("ppmd8 decompress: {e}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_text() {
        let input = b"the quick brown fox jumps over the lazy dog. ".repeat(1000);
        let codec = Ppmd8Codec::new();
        let compressed = codec.compress(&input).expect("compress");
        let decompressed = codec
            .decompress(&compressed, input.len() as u32)
            .expect("decompress");
        assert_eq!(decompressed, input);
    }
}
