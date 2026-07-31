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
//! | 0x02 | zstd | Zstandard frame format; better ratio than LZ4 |
//! | 0x03 | xz | XZ/LZMA2 format; best ratio for binary data |
//!
//! Other codec ids are rejected with [`CoreError::UnsupportedFeature`].

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::io::Read;

use crate::error::CoreError;

/// Codec id 0x00: store (no compression).
pub const CODEC_STORE: u8 = 0x00;
/// Codec id 0x01: LZ4 block format.
pub const CODEC_LZ4: u8 = 0x01;
/// Codec id 0x02: Zstandard frame format.
pub const CODEC_ZSTD: u8 = 0x02;
/// Codec id 0x03: XZ/LZMA2 format.
pub const CODEC_XZ: u8 = 0x03;

/// Default ZSTD compression level for the deepening phase.
/// Level 9 gives near-LZMA compression ratios at LZ4-class speed.
pub const ZSTD_DEFAULT_LEVEL: i32 = 9;

/// Default XZ preset for the deepening phase.
/// Preset 6 is the XZ default level — good ratio without excessive
/// encoding time. LZMA2 typically beats ZSTD by 10-20% on binary
/// and structured data.
pub const XZ_DEFAULT_PRESET: u32 = 6;

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
        CODEC_ZSTD => compress_zstd(plaintext, ZSTD_DEFAULT_LEVEL),
        CODEC_XZ => compress_xz(plaintext, XZ_DEFAULT_PRESET),
        other => Err(CoreError::UnsupportedFeature {
            feature: format!(
                "compress codec 0x{other:02X} (supported: store=0x00, lz4=0x01, zstd=0x02, xz=0x03)"
            ),
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
        CODEC_ZSTD => {
            let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
                reason: format!("decompress: expected_len {expected_len} exceeds usize"),
            })?;
            let result = zstd::decode_all(compressed).map_err(|e| CoreError::Corrupt {
                reason: format!("zstd decompress failed: {e}"),
            })?;
            if result.len() != expected_us {
                return Err(CoreError::Corrupt {
                    reason: format!(
                        "zstd decompress: result length {} does not match plaintext_len {expected_us}",
                        result.len()
                    ),
                });
            }
            Ok(result)
        }
        CODEC_XZ => {
            let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
                reason: format!("decompress: expected_len {expected_len} exceeds usize"),
            })?;
            let mut decoder = xz2::read::XzDecoder::new(compressed);
            let mut result = Vec::with_capacity(expected_us);
            decoder.read_to_end(&mut result).map_err(|e| CoreError::Corrupt {
                reason: format!("xz decompress failed: {e}"),
            })?;
            if result.len() != expected_us {
                return Err(CoreError::Corrupt {
                    reason: format!(
                        "xz decompress: result length {} does not match `plaintext_len` {expected_us}",
                        result.len()
                    ),
                });
            }
            Ok(result)
        }
        other => Err(CoreError::UnsupportedFeature {
            feature: format!(
                "decompress codec 0x{other:02X} (supported: store=0x00, lz4=0x01, zstd=0x02, xz=0x03)"
            ),
        }),
    }
}

/// Compress with LZ4, prepending the original size as a 4-byte LE
/// header (the format `lz4_flex::decompress_size_prepended` expects).
#[must_use]
pub fn compress_lz4_with_size(plaintext: &[u8]) -> Vec<u8> {
    lz4_flex::compress_prepend_size(plaintext)
}

/// Compress with Zstandard at the given level (1-22). Level 9 gives
/// near-LZMA compression ratios at LZ4-class speed. Prepends no size
/// header; the `plaintext_len` from the drop record is used on decompress.
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if the ZSTD encoder fails (e.g.
/// out of memory).
pub fn compress_zstd(plaintext: &[u8], level: i32) -> Result<Vec<u8>, CoreError> {
    zstd::encode_all(plaintext, level).map_err(|e| CoreError::Corrupt {
        reason: format!("zstd compress failed: {e}"),
    })
}

/// Compress with XZ/LZMA2 at the given preset (0-9). Preset 6 is the
/// default; LZMA2 typically gives 10-20% better ratio than ZSTD on
/// binary and structured data. The `plaintext_len` from the drop
/// record is used on decompress (no size header in the XZ stream).
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if the XZ encoder fails.
pub fn compress_xz(plaintext: &[u8], preset: u32) -> Result<Vec<u8>, CoreError> {
    use std::io::Write;
    let mut encoder = xz2::write::XzEncoder::new(Vec::new(), preset);
    encoder
        .write_all(plaintext)
        .map_err(|e| CoreError::Corrupt {
            reason: format!("xz compress (write) failed: {e}"),
        })?;
    encoder.finish().map_err(|e| CoreError::Corrupt {
        reason: format!("xz compress (finish) failed: {e}"),
    })
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
