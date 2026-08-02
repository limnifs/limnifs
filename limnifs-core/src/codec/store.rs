//! Store codec (0x00): no compression. Bytes are written verbatim.

use crate::codec::Codec;
use crate::error::CoreError;

/// Store codec: the identity function.
pub struct StoreCodec;

impl Codec for StoreCodec {
    fn id(&self) -> u8 {
        super::CODEC_STORE
    }

    fn name(&self) -> &'static str {
        "store"
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        Ok(plaintext.to_vec())
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let expected = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
            reason: format!("decompress: expected_len {expected_len} exceeds usize"),
        })?;
        if compressed.len() != expected {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "store codec: compressed length {} does not match plaintext_len {expected}",
                    compressed.len()
                ),
            });
        }
        Ok(compressed.to_vec())
    }
}
