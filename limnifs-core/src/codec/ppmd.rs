//! PPMd codecs (ids 0x0C PPMd7, 0x12 PPMd8): Prediction by Partial Matching.
//!
//! Wraps `omnizip-ppmd` 0.13. Both variants expose a user-tunable
//! memory budget — more memory = larger context trie = better ratio.
//!
//! - PPMd7 (0x0C): default 80 MB budget, byte-level PPM with PPM\*C escape
//! - PPMd8 (0x12): default 64 MB budget, RESTART restoration + RLE
//!
//! Clean-room implementation from Shkarin's DCC 2001 paper.
//! No LGPL code in the source tree.

use crate::codec::{Codec, CodecTunables, PerCodecTunables, CODEC_PPMD};
use crate::error::CoreError;

/// Default PPMd7 memory budget: 80 MB.
pub const DEFAULT_PPMD7_BUDGET: usize = 80 * 1024 * 1024;
/// Default PPMd7 context order.
pub const DEFAULT_PPMD7_ORDER: u8 = 4;

/// Strongly-typed PPMd7 tunables. Implements `PerCodecTunables`
/// for clean OCP — adding a new PPMd knob is one field here, no
/// edits to the flat `CodecTunables` struct.
#[derive(Clone, Debug)]
pub struct Ppmd7Tunables {
    pub order: u8,
    pub budget: usize,
}

impl Default for Ppmd7Tunables {
    fn default() -> Self {
        Self {
            order: DEFAULT_PPMD7_ORDER,
            budget: DEFAULT_PPMD7_BUDGET,
        }
    }
}

/// PPMd7 codec (id 0x0C).
pub struct PpmdCodec {
    order: u8,
    budget: usize,
}

impl PpmdCodec {
    #[must_use]
    pub fn new() -> Self {
        Self {
            order: DEFAULT_PPMD7_ORDER,
            budget: DEFAULT_PPMD7_BUDGET,
        }
    }

    /// Set a custom memory budget. Larger budget = better ratio.
    #[must_use]
    pub fn with_budget(mut self, budget: usize) -> Self {
        self.budget = budget;
        self
    }

    /// Set the context model order (1..=16).
    #[must_use]
    pub fn with_order(mut self, order: u8) -> Self {
        self.order = order;
        self
    }
}

impl Default for PpmdCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec for PpmdCodec {
    fn id(&self) -> u8 {
        CODEC_PPMD
    }
    fn name(&self) -> &'static str {
        "ppmd7"
    }

    fn min_compress_size(&self) -> usize {
        4096
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        omnizip_ppmd::ppmd7::compress_with_budget(plaintext, self.order, self.budget).map_err(|e| {
            CoreError::Corrupt {
                reason: format!("ppmd7 compress: {e}"),
            }
        })
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let expected = usize::try_from(expected_len).unwrap_or(usize::MAX);
        omnizip_ppmd::ppmd7::decompress(compressed, expected).map_err(|e| CoreError::Corrupt {
            reason: format!("ppmd7 decompress: {e}"),
        })
    }

    fn compress_with_tunables(
        &self,
        plaintext: &[u8],
        t: &CodecTunables,
    ) -> Result<Vec<u8>, CoreError> {
        let order = if t.ppmd_order > 0 {
            t.ppmd_order
        } else {
            self.order
        };
        let budget = if t.ppmd7_budget > 0 {
            t.ppmd7_budget
        } else {
            self.budget
        };
        omnizip_ppmd::ppmd7::compress_with_budget(plaintext, order, budget).map_err(|e| {
            CoreError::Corrupt {
                reason: format!("ppmd7 compress: {e}"),
            }
        })
    }
}

impl PerCodecTunables for PpmdCodec {
    type Tunables = Ppmd7Tunables;

    fn compress_with_owned_tunables(
        &self,
        plaintext: &[u8],
        t: &Self::Tunables,
    ) -> Result<Vec<u8>, CoreError> {
        omnizip_ppmd::ppmd7::compress_with_budget(plaintext, t.order, t.budget).map_err(|e| {
            CoreError::Corrupt {
                reason: format!("ppmd7 compress: {e}"),
            }
        })
    }
}

#[cfg(test)]
mod per_codec_tunables_tests {
    use super::*;

    #[test]
    fn owned_tunables_round_trips() {
        let plaintext = b"the quick brown fox jumps over the lazy dog. ".repeat(50);
        let codec = PpmdCodec::new();
        let t = Ppmd7Tunables {
            order: 6,
            budget: 16 * 1024 * 1024,
        };
        let compressed = codec
            .compress_with_owned_tunables(&plaintext, &t)
            .expect("compress");
        let recovered = codec
            .decompress(&compressed, plaintext.len() as u32)
            .expect("decompress");
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn bigger_owned_budget_helps_ratio() {
        let mut input = Vec::with_capacity(1024 * 1024);
        let paragraph = b"the quick brown fox jumps over the lazy dog. ";
        while input.len() + paragraph.len() <= 1024 * 1024 {
            input.extend_from_slice(paragraph);
        }
        let codec = PpmdCodec::new();
        let small = Ppmd7Tunables {
            order: 4,
            budget: 8 * 1024 * 1024,
        };
        let big = Ppmd7Tunables {
            order: 4,
            budget: 256 * 1024 * 1024,
        };
        let small_c = codec
            .compress_with_owned_tunables(&input, &small)
            .expect("small");
        let big_c = codec
            .compress_with_owned_tunables(&input, &big)
            .expect("big");
        assert!(
            big_c.len() <= small_c.len(),
            "256MB budget ({}) should not be worse than 8MB ({})",
            big_c.len(),
            small_c.len()
        );
    }
}
