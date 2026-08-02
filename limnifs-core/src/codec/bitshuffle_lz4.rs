//! Bitshuffle+LZ4 codec (id 0x0F): bit-level shuffle + LZ4 back-end.
//!
//! Unlike byte-shuffle (which transposes bytes within blocks of N
//! bytes), bit-shuffle transposes bits within blocks. Better for
//! floating-point arrays where each bit plane has different
//! statistical properties.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::codec::{compress, decompress, Codec, CODEC_BITSHUFFLE_LZ4, CODEC_LZ4};
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
    pub fn with_item_size(item_size: usize) -> Self {
        Self {
            item_size: if item_size == 0 { DEFAULT_ITEM_SIZE } else { item_size },
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
        let shuffled = filter.encode(plaintext);
        let lz4_compressed = compress(CODEC_LZ4, &shuffled)?;
        let shuffled_len = u32::try_from(shuffled.len()).unwrap_or(u32::MAX);
        let mut out = Vec::with_capacity(4 + lz4_compressed.len());
        out.extend_from_slice(&shuffled_len.to_le_bytes());
        out.extend_from_slice(&lz4_compressed);
        Ok(out)
    }

    fn decompress(&self, compressed: &[u8], _expected_len: u32) -> Result<Vec<u8>, CoreError> {
        if compressed.len() < 4 {
            return Err(CoreError::Corrupt {
                reason: "bitshuffle+lz4: input too short for length prefix".into(),
            });
        }
        let shuffled_len = u32::from_le_bytes([
            compressed[0], compressed[1], compressed[2], compressed[3],
        ]) as usize;
        let lz4_bytes = &compressed[4..];
        let shuffled = decompress(CODEC_LZ4, lz4_bytes, shuffled_len as u32)?;
        let filter = omnizip_filters::shuffle::BitShuffle::new(self.item_size);
        let plaintext = filter.decode(&shuffled);
        Ok(plaintext)
    }
}
