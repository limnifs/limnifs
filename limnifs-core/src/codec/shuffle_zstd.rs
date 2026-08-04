//! Shuffle+Zstd codec (id 0x0E): byte-shuffle filter + Zstd back-end.
//!
//! Same concept as Shuffle+LZ4 (0x0A) but uses Zstd for the back-end,
//! giving better ratio at slightly higher CPU cost. Best for
//! floating-point arrays and structured numeric data where the
//! shuffle filter exposes redundancy that Zstd exploits.

use crate::codec::{Codec, CODEC_SHUFFLE_ZSTD, CODEC_ZSTD};
use crate::error::CoreError;
use omnizip_filters::Filter;

/// Default item size for the byte-shuffle filter (bytes per element).
const DEFAULT_ITEM_SIZE: usize = 4;

pub struct ShuffleZstdCodec {
    item_size: usize,
}

impl ShuffleZstdCodec {
    #[must_use]
    pub fn new() -> Self {
        Self {
            item_size: DEFAULT_ITEM_SIZE,
        }
    }

    #[must_use]
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

impl Default for ShuffleZstdCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec for ShuffleZstdCodec {
    fn id(&self) -> u8 {
        CODEC_SHUFFLE_ZSTD
    }

    fn name(&self) -> &'static str {
        "shuffle+zstd"
    }

    fn min_compress_size(&self) -> usize {
        512
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        let filter = omnizip_filters::shuffle::ByteShuffle::new(self.item_size);
        crate::codec::composite::filter_then_compress(plaintext, &filter, CODEC_ZSTD)
    }

    fn decompress(&self, compressed: &[u8], _expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let filter = omnizip_filters::shuffle::ByteShuffle::new(self.item_size);
        crate::codec::composite::decompress_then_filter(
            compressed,
            &filter,
            CODEC_ZSTD,
            "shuffle+zstd",
        )
    }
}
