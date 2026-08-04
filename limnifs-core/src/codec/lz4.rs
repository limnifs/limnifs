//! LZ4 codecs (0x01 fast, 0x13 high-compression) via `omnizip-lz4` 0.14.
//!
//! Both variants share the same block wire format (4-byte LE
//! original-size prefix + LZ4 block). Decode is identical; the
//! difference is the encoder:
//!
//! - **LZ4 fast** (0x01): hash-table match finder, greedy parsing.
//!   ~1 GB/s encode, moderate ratio.
//! - **LZ4 HC** (0x13): hash-chain match finder + lazy parsing with
//!   look-ahead (in-house omnizip-rs encoder, `omnizip-lz4 0.14.1`).
//!   2–3× better ratio at 5–10× slower encode. Decode speed identical.
//!
//! ## Why HC was dormant until 0.14.1
//!
//! `omnizip-lz4 0.13.1` shipped `Lz4HcCodec` whose body was identical
//! to `Lz4FastCodec` — both called `lz4_flex::compress_prepend_size`.
//! LimniFS filed proposal #1 (see `docs/omnizip-proposals/lz4-hc.md`)
//! and omnizip-rs implemented a real hash-chain HC encoder that
//! landed in 0.14.1.

use crate::codec::Codec;
use crate::error::CoreError;

/// LZ4 fast codec. Wraps `lz4_flex::compress_prepend_size`.
pub struct Lz4Codec;

impl Codec for Lz4Codec {
    fn id(&self) -> u8 {
        super::CODEC_LZ4
    }

    fn name(&self) -> &'static str {
        "lz4"
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        Ok(compress_lz4_with_size(plaintext))
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        decompress_lz4_prepended(compressed, expected_len)
    }
}

/// LZ4 HC codec (0x13). Hash-chain match finder + lazy parsing.
///
/// Decode is byte-compatible with [`Lz4Codec`] (same wire format);
/// only the encoder differs.
pub struct Lz4HcCodec;

impl Codec for Lz4HcCodec {
    fn id(&self) -> u8 {
        super::CODEC_LZ4_HC
    }

    fn name(&self) -> &'static str {
        "lz4-hc"
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        // Delegate to omnizip-lz4 0.14.1's Lz4HcCodec, which wraps
        // the in-house hash-chain HC encoder.
        let codec = omnizip_lz4::Lz4HcCodec;
        omnizip_codecs::Codec::compress(
            &codec,
            plaintext,
            omnizip_codecs::CompressionLevel::default(),
        )
        .map_err(lz4_hc_err)
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        // Same wire format as fast LZ4.
        decompress_lz4_prepended(compressed, expected_len)
    }
}

fn lz4_hc_err(e: omnizip_codecs::OmnizipError) -> CoreError {
    CoreError::Corrupt {
        reason: format!("lz4-hc: {e}"),
    }
}

/// Decompress an LZ4 block with the 4-byte LE size prefix. Shared by
/// fast and HC variants.
fn decompress_lz4_prepended(compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
    let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
        reason: format!("decompress: expected_len {expected_len} exceeds usize"),
    })?;
    let result =
        lz4_flex::decompress_size_prepended(compressed).map_err(|e| CoreError::Corrupt {
            reason: format!("lz4 decompress failed: {e}"),
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

/// Compress with LZ4 fast, prepending the original size as a 4-byte LE
/// header (the format `lz4_flex::decompress_size_prepended` expects).
#[must_use]
pub fn compress_lz4_with_size(plaintext: &[u8]) -> Vec<u8> {
    lz4_flex::compress_prepend_size(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hc_beats_fast_on_non_rle_friendly_input() {
        // Mixed text where the HC match finder finds longer matches
        // than greedy fast LZ4.
        let mut input = Vec::new();
        let paragraph = b"the quick brown fox jumps over the lazy dog. ";
        let mut i = 0u32;
        while input.len() < 50_000 {
            input.extend_from_slice(format!("{i:04}: {paragraph:?}\n").as_bytes());
            i += 1;
        }
        let fast = Lz4Codec.compress(&input).expect("fast");
        let hc = Lz4HcCodec.compress(&input).expect("hc");
        assert!(
            hc.len() < fast.len(),
            "LZ4 HC ({}) should beat fast ({}) on mixed text",
            hc.len(),
            fast.len()
        );
    }

    #[test]
    fn hc_decodes_through_fast_decoder() {
        // Same wire format: HC output must decode through the fast decoder.
        let input = b"hello world. hello world. hello world.".repeat(20);
        let hc = Lz4HcCodec.compress(&input).expect("hc compress");
        let recovered = Lz4Codec
            .decompress(&hc, input.len() as u32)
            .expect("cross-decode");
        assert_eq!(recovered, input);
    }

    #[test]
    fn hc_round_trips() {
        let input = b"the quick brown fox jumps over the lazy dog. ".repeat(50);
        let hc = Lz4HcCodec.compress(&input).expect("hc");
        let recovered = Lz4HcCodec
            .decompress(&hc, input.len() as u32)
            .expect("decompress");
        assert_eq!(recovered, input);
    }
}
