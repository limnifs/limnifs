//! Snappy codec (0x06): Google's high-speed format via the `omnizip-snappy`
//! crate (wrapping the pure-Rust `snap` implementation).
//!
//! Snappy prioritises speed over ratio: encode and decode are both
//! ~500 MB/s, with moderate compression. Used in Parquet, ORC, Avro,
//! and `SQLite` WAL files. No compression levels.
//!
//! This is the first codec in limnifs that delegates to the omnizip-rs
//! workspace — the integration pattern for future codecs (LZMA, ZSTD)
//! once their omnizip-rs ports are complete.

use crate::codec::Codec;
use crate::error::CoreError;
use omnizip_codecs::Codec as OmnizipCodec;

/// Snappy codec. No compression levels.
pub struct SnappyCodec;

impl Codec for SnappyCodec {
    fn id(&self) -> u8 {
        super::CODEC_SNAPPY
    }

    fn name(&self) -> &'static str {
        "snappy"
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        let inner = omnizip_snappy::SnappyCodec;
        inner
            .compress(plaintext, omnizip_codecs::CompressionLevel::default())
            .map_err(|e| CoreError::Corrupt {
                reason: format!("snappy compress failed: {e}"),
            })
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let inner = omnizip_snappy::SnappyCodec;
        inner
            .decompress(compressed, expected_len)
            .map_err(|e| CoreError::Corrupt {
                reason: format!("snappy decompress failed: {e}"),
            })
    }
}
