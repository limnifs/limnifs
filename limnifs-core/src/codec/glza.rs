//! GLZA codec (id 0x0D): grammar-based LZ compression.
//!
//! Wraps `omnizip-glza` 0.11. Phase 2: Huffman entropy coding of
//! the grammar rule stream. Validated ratio: 3.48% on repetitive
//! data (500 KB). Excels on hierarchically repetitive data (XML,
//! DNA, logs).
//!
//! Grammar construction is O(n²) — avoid routing large non-repetitive
//! files (>512 KB) to GLZA. The file categorizer should enforce a
//! size cap when adding a DNA/genome categorizer.
//!
//! Clean-room implementation from Gregory Smith's published format
//! specification. No GPL code in the source tree.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::codec::Codec;
use crate::codec::CODEC_GLZA;
use crate::error::CoreError;

pub struct GlzaCodec;

impl Codec for GlzaCodec {
    fn id(&self) -> u8 {
        CODEC_GLZA
    }
    fn name(&self) -> &'static str {
        "glza"
    }

    fn min_compress_size(&self) -> usize {
        4096
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        omnizip_glza::compress(plaintext).map_err(|e| CoreError::Corrupt {
            reason: format!("glza compress: {e}"),
        })
    }

    fn decompress(&self, compressed: &[u8], _expected_len: u32) -> Result<Vec<u8>, CoreError> {
        omnizip_glza::decompress(compressed).map_err(|e| CoreError::Corrupt {
            reason: format!("glza decompress: {e}"),
        })
    }
}
