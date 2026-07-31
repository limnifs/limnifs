//! Codec registry — dispatches compression/decompression by codec id.
//!
//! Each drop record carries a `representation` triple `(codec, aead, ec)`.
//! This module centralises the codec dispatch so the slab reader and
//! the deepening stage share a single source of truth for which codecs
//! are supported and how to invoke them.
//!
//! ## Supported codecs
//!
//! | Id  | Name | Encode | Decode | Notes |
//! |-----|------|--------|--------|-------|
//! | 0x00 | store | yes (identity) | yes | No compression |
//! | 0x01 | lz4   | yes ([`lz4_flex`]) | yes | Fast baseline; pure Rust |
//! | 0x02 | zstd  | yes ([`ruzstd`] `Fastest`) | yes ([`ruzstd`]) | Pure Rust; ZSTD level 1 |
//! | 0x03 | xz    | **no** | yes ([`lzma-rs`]) | Decode-only for legacy drops |
//!
//! **Why XZ is decode-only.** [`lzma-rs`] 0.3.0 ships an LZMA2 "encoder" that
//! wraps input as uncompressed chunks (`encode/lzma2.rs`) and a raw-LZMA
//! encoder that emits literals only (`encode/dumbencoder.rs`). Neither
//! performs real compression. There is no mature pure-Rust LZMA encoder as
//! of 2026, so `LimniFS` reserves the XZ codec id for reading legacy drops
//! produced by external tooling and routes its own encoding to ZSTD.
//!
//! **100% pure Rust.** No C libraries. Air-gapped safe.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::io::Read;

use crate::error::CoreError;

/// Codec id 0x00: store (no compression).
pub const CODEC_STORE: u8 = 0x00;
/// Codec id 0x01: LZ4 block format ([`lz4_flex`], pure Rust).
pub const CODEC_LZ4: u8 = 0x01;
/// Codec id 0x02: Zstandard frame format ([`ruzstd`], pure Rust).
/// Encode uses `CompressionLevel::Fastest` (ZSTD level 1); decode supports
/// any level the reference encoder can produce.
pub const CODEC_ZSTD: u8 = 0x02;
/// Codec id 0x03: XZ/LZMA2 format. Decode-only in pure Rust ([`lzma-rs`]).
pub const CODEC_XZ: u8 = 0x03;

/// Returns the best available codec for compressible content classes.
/// ZSTD level 1 beats LZ4 on ratio at similar encode speed and round-trips
/// through a pure-Rust encoder/decoder pair.
#[must_use]
pub fn best_compressible_codec() -> u8 {
    CODEC_ZSTD
}

/// Returns the best available codec for binary content classes.
/// ZSTD handles structured binary data well at level 1; higher-ratio
/// pure-Rust options do not exist as of 2026.
#[must_use]
pub fn best_binary_codec() -> u8 {
    CODEC_ZSTD
}

/// Compress `plaintext` using the codec identified by `codec_id`.
///
/// # Errors
///
/// Returns [`CoreError::UnsupportedFeature`] for unknown codec ids and for
/// codecs that are decode-only in pure Rust (currently `CODEC_XZ`).
/// Returns [`CoreError::Corrupt`] if the encoder fails.
pub fn compress(codec_id: u8, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    match codec_id {
        CODEC_STORE => Ok(plaintext.to_vec()),
        CODEC_LZ4 => Ok(lz4_flex::compress_prepend_size(plaintext)),
        CODEC_ZSTD => compress_zstd(plaintext),
        CODEC_XZ => Err(CoreError::UnsupportedFeature {
            feature: "compress codec 0x03 (xz): pure-Rust LZMA encoder does not exist; \
                      lzma-rs 0.3.0's encoder is a non-compressing stub"
                .to_string(),
        }),
        other => Err(CoreError::UnsupportedFeature {
            feature: format!(
                "compress codec 0x{other:02X} (supported: store=0x00, lz4=0x01, zstd=0x02; xz=0x03 decode-only)"
            ),
        }),
    }
}

/// Decompress `compressed` using the codec identified by `codec_id`.
/// The `expected_len` is the `plaintext_len` from the drop record;
/// the decompressed output MUST match it exactly.
///
/// # Errors
///
/// Returns [`CoreError::UnsupportedFeature`] for unknown codec ids.
/// Returns [`CoreError::Corrupt`] if decompression fails or the
/// result length does not match `expected_len`.
pub fn decompress(
    codec_id: u8,
    compressed: &[u8],
    expected_len: u32,
) -> Result<Vec<u8>, CoreError> {
    match codec_id {
        CODEC_STORE => {
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
        CODEC_LZ4 => {
            let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
                reason: format!("decompress: expected_len {expected_len} exceeds usize"),
            })?;
            let result = lz4_flex::decompress_size_prepended(compressed).map_err(|e| {
                CoreError::Corrupt {
                    reason: format!("lz4 decompress failed: {e}"),
                }
            })?;
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
            let decoder =
                ruzstd::decoding::StreamingDecoder::new(compressed).map_err(|e| {
                    CoreError::Corrupt {
                        reason: format!("zstd decompress (init) failed: {e}"),
                    }
                })?;
            let mut result = Vec::with_capacity(expected_us);
            decoder
                .take(u64::from(expected_len))
                .read_to_end(&mut result)
                .map_err(|e| CoreError::Corrupt {
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
            let mut result = Vec::with_capacity(expected_us);
            lzma_rs::lzma2_decompress(&mut std::io::Cursor::new(compressed), &mut result)
                .map_err(|e| CoreError::Corrupt {
                    reason: format!("lzma2 decompress failed: {e}"),
                })?;
            if result.len() != expected_us {
                return Err(CoreError::Corrupt {
                    reason: format!(
                        "lzma2 decompress: result length {} does not match plaintext_len {expected_us}",
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

/// Compress with Zstandard via [`ruzstd`] at `CompressionLevel::Fastest`
/// (ZSTD level 1). The output is a standard ZSTD frame decodable by any
/// conformant ZSTD decoder.
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if the ZSTD encoder fails.
pub fn compress_zstd(plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    // ruzstd::encoding::compress_to_vec returns Vec<u8> infallibly; the only
    // failure mode is I/O on the Vec writer, which cannot fail.
    Ok(ruzstd::encoding::compress_to_vec(
        plaintext,
        ruzstd::encoding::CompressionLevel::Fastest,
    ))
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
            "lz4 should compress repetitive data: {} vs {}",
            compressed.len(),
            data.len()
        );
    }

    #[test]
    fn zstd_round_trips() {
        let data = b"The quick brown fox jumps over the lazy dog. ".repeat(1000);
        let compressed = compress_zstd(&data).expect("zstd compress");
        let decompressed = decompress(
            CODEC_ZSTD,
            &compressed,
            u32::try_from(data.len()).expect("fits u32"),
        )
        .expect("zstd decompress");
        assert_eq!(decompressed, data);
    }

    #[test]
    fn zstd_compresses_repetitive_data() {
        let data = vec![0x41u8; 10_000];
        let compressed = compress_zstd(&data).expect("zstd compress");
        assert!(
            compressed.len() < data.len(),
            "zstd should compress repetitive data: {} vs {}",
            compressed.len(),
            data.len()
        );
    }

    #[test]
    fn zstd_compresses_better_than_lz4_on_text() {
        let data = b"The quick brown fox. ".repeat(10_000);
        let lz4 = compress(CODEC_LZ4, &data).expect("lz4");
        let zstd = compress_zstd(&data).expect("zstd");
        assert!(
            zstd.len() < lz4.len(),
            "zstd ({}) should be smaller than lz4 ({}) on text",
            zstd.len(),
            lz4.len()
        );
    }

    #[test]
    fn zstd_compresses_binary_data() {
        let data: Vec<u8> = (0..100_000u32).map(|i| u8::try_from(i % 256).expect("fits u8")).collect();
        let compressed = compress_zstd(&data).expect("zstd compress");
        assert!(compressed.len() < data.len());
        let decompressed = decompress(
            CODEC_ZSTD,
            &compressed,
            u32::try_from(data.len()).expect("fits u32"),
        )
        .expect("zstd decompress");
        assert_eq!(decompressed, data);
    }

    #[test]
    fn xz_encode_is_unsupported() {
        match compress(CODEC_XZ, b"data") {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("non-compressing stub"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn reject_unknown_codec() {
        let result = compress(0xFF, b"data");
        assert!(matches!(result, Err(CoreError::UnsupportedFeature { .. })));
    }
}
