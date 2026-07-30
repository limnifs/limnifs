//! Codec registry — dispatches compression/decompression by codec id.
//!
//! Each drop record carries a `representation` triple `(codec, aead, ec)`.
//! This module centralises the codec dispatch so the slab reader and
//! the deepening stage share a single source of truth for which codecs
//! are supported and how to invoke them.
//!
//! ## Supported codecs (v0.1)
//!
//! | Id | Name | Notes |
//! |---|---|---|
//! | 0x00 | store | No compression; bytes are plaintext |
//! | 0x01 | lz4 | LZ4 block format (no frame); mandatory baseline |
//!
//! Other codec ids are rejected with [`CoreError::UnsupportedFeature`].

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::error::CoreError;

/// Codec id 0x00: store (no compression).
pub const CODEC_STORE: u8 = 0x00;
/// Codec id 0x01: LZ4 block format.
pub const CODEC_LZ4: u8 = 0x01;

/// Compress `plaintext` using the codec identified by `codec_id`.
///
/// For `CODEC_STORE` the input is returned unchanged. For `CODEC_LZ4`
/// the input is compressed with LZ4 block format (no frame header).
///
/// # Errors
///
/// Returns [`CoreError::UnsupportedFeature`] for unknown codec ids.
pub fn compress(codec_id: u8, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    match codec_id {
        CODEC_STORE => Ok(plaintext.to_vec()),
        CODEC_LZ4 => Ok(lz4_flex::compress_prepend_size(plaintext)),
        other => Err(CoreError::UnsupportedFeature {
            feature: format!("compress codec 0x{other:02X} (supported: store=0x00, lz4=0x01)"),
        }),
    }
}

/// Decompress `compressed` using the codec identified by `codec_id`.
/// The `expected_len` is the `plaintext_len` from the drop record;
/// the decompressed output MUST match it exactly.
///
/// For `CODEC_STORE` the input is returned unchanged. For `CODEC_LZ4`
/// the input is decompressed with LZ4 block format, then its length
/// is verified against `expected_len`.
///
/// # Errors
///
/// Returns [`CoreError::UnsupportedFeature`] for unknown codec ids.
/// Returns [`CoreError::Corrupt`] if LZ4 decompression fails or the
/// result length does not match `expected_len`.
pub fn decompress(
    codec_id: u8,
    compressed: &[u8],
    expected_len: u32,
) -> Result<Vec<u8>, CoreError> {
    match codec_id {
        CODEC_STORE => {
            let actual = compressed.len();
            let expected = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
                reason: format!("decompress: expected_len {expected_len} exceeds usize"),
            })?;
            if actual != expected {
                return Err(CoreError::Corrupt {
                    reason: format!(
                        "store codec: compressed length {actual} does not match plaintext_len {expected}"
                    ),
                });
            }
            Ok(compressed.to_vec())
        }
        CODEC_LZ4 => {
            let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
                reason: format!("decompress: expected_len {expected_len} exceeds usize"),
            })?;
            let result = lz4_flex::decompress_size_prepended(compressed).map_err(|e| {
                CoreError::Corrupt {
                    reason: format!("lz4 decompress failed: {e}"),
                }
            })?;
            // The prepended size header carries the original length.
            // Verify it matches the drop record's plaintext_len.
            if result.len() != expected_us {
                return Err(CoreError::Corrupt {
                    reason: format!(
                        "lz4 decompress: result length {} does not match plaintext_len {expected_us}",
                        result.len()
                    ),
                });
            }
            Ok(result)
        }
        other => Err(CoreError::UnsupportedFeature {
            feature: format!("decompress codec 0x{other:02X} (supported: store=0x00, lz4=0x01)"),
        }),
    }
}

/// Compress with LZ4, prepending the original size as a 4-byte LE
/// header (the format `lz4_flex::decompress_size_prepended` expects).
#[must_use]
pub fn compress_lz4_with_size(plaintext: &[u8]) -> Vec<u8> {
    lz4_flex::compress_prepend_size(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_compress_is_identity() {
        let data = b"hello world";
        let compressed = compress(CODEC_STORE, data).expect("store compress");
        assert_eq!(compressed, data);
    }

    #[test]
    fn store_decompress_validates_length() {
        let data = b"hello world";
        let result = decompress(CODEC_STORE, data, 11).expect("store decompress");
        assert_eq!(result, data);
    }

    #[test]
    fn store_decompress_rejects_length_mismatch() {
        let data = b"hello world";
        match decompress(CODEC_STORE, data, 99) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("does not match"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn lz4_round_trips() {
        let data = b"Lorem ipsum dolor sit amet, consectetur adipiscing elit. \
                    Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.";
        let compressed = compress(CODEC_LZ4, data).expect("lz4 compress");
        let decompressed = decompress(
            CODEC_LZ4,
            &compressed,
            u32::try_from(data.len()).expect("fits u32"),
        )
        .expect("lz4 decompress");
        assert_eq!(decompressed, data);
    }

    #[test]
    fn lz4_compresses_repetitive_data() {
        let data = vec![0x41u8; 10_000];
        let compressed = compress(CODEC_LZ4, &data).expect("lz4 compress");
        assert!(
            compressed.len() < data.len(),
            "lz4 should compress repetitive data: compressed={} original={}",
            compressed.len(),
            data.len()
        );
    }

    #[test]
    fn rejects_unknown_compress_codec() {
        match compress(0xFF, b"data") {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("0xFF"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_decompress_codec() {
        match decompress(0xFF, b"data", 4) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("0xFF"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn lz4_decompress_rejects_corrupt_input() {
        let garbage = vec![0xFFu8; 100];
        match decompress(CODEC_LZ4, &garbage, 1000) {
            Err(CoreError::Corrupt { .. }) => {}
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn compress_lz4_with_size_prepends_length() {
        let data = b"test data for lz4";
        let compressed = compress_lz4_with_size(data);
        // First 4 bytes should be the original length (LE).
        let size = u32::from_le_bytes([compressed[0], compressed[1], compressed[2], compressed[3]]);
        assert_eq!(size as usize, data.len());
    }
}
