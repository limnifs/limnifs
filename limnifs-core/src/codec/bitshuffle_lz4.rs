//! Bitshuffle+LZ4 codec (id 0x0F): bit-level shuffle + LZ4 back-end.
//!
//! Unlike byte-shuffle (which transposes bytes within blocks of N
//! bytes), bit-shuffle transposes bits within blocks. Better for
//! floating-point arrays where each bit plane has different
//! statistical properties.

use crate::codec::{Codec, CODEC_BITSHUFFLE_LZ4, CODEC_LZ4};
use crate::error::CoreError;
use omnizip_filters::Filter;

const DEFAULT_ITEM_SIZE: usize = 8;

pub struct BitshuffleLz4Codec {
    item_size: usize,
}

impl BitshuffleLz4Codec {
    #[must_use]
    pub fn new() -> Self {
        Self {
            item_size: DEFAULT_ITEM_SIZE,
        }
    }

    #[must_use]
    #[allow(dead_code)]
    #[allow(dead_code)]
    pub fn with_item_size(item_size: usize) -> Self {
        Self {
            item_size: if item_size == 0 {
                DEFAULT_ITEM_SIZE
            } else {
                item_size
            },
        }
    }
}

impl Default for BitshuffleLz4Codec {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec for BitshuffleLz4Codec {
    fn id(&self) -> u8 {
        CODEC_BITSHUFFLE_LZ4
    }

    fn name(&self) -> &'static str {
        "bitshuffle+lz4"
    }

    fn min_compress_size(&self) -> usize {
        512
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        let filter = omnizip_filters::shuffle::BitShuffle::new(self.item_size);
        crate::codec::composite::filter_then_compress(plaintext, &filter, CODEC_LZ4)
    }

    fn decompress(&self, compressed: &[u8], _expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let filter = omnizip_filters::shuffle::BitShuffle::new(self.item_size);
        crate::codec::composite::decompress_then_filter(
            compressed,
            &filter,
            CODEC_LZ4,
            "bitshuffle+lz4",
        )
    }
}
