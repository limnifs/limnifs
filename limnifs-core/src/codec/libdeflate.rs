//! libdeflate codec (0x14): pure-Rust RFC 1951 via `omnizip-libdeflate`.
//!
//! `omnizip-libdeflate` is omnizip's in-house port of libdeflate
//! (LZ77 + fixed-Huffman encode, canonical Huffman inflate). The wire
//! format is byte-compatible with [`crate::codec::DeflateCodec`] (0x05)
//! — both are RFC 1951 DEFLATE wrapped in RFC 1950 zlib — but the
//! implementation differs:
//!
//! - **0x05 (DeflateCodec)**: wraps `miniz_oxide`. Dynamic-Huffman
//!   encoder, mature ecosystem. Better ratio on most inputs.
//! - **0x14 (LibdeflateCodec)**: omnizip's pure-Rust port. Stored
//!   blocks for tiny inputs, LZ77 + fixed-Huffman otherwise. No
//!   dynamic-Huffman encode yet (Phase 1 per omnizip's roadmap).
//!   Targets decode speed; encode ratio is currently worse than 0x05
//!   on compressible inputs.
//!
//! ## Known upstream bug: Adler-32 trailer
//!
//! `omnizip-libdeflate` 0.14.6 computes the zlib trailer's Adler-32
//! over the COMPRESSED stream rather than the original plaintext,
//! violating RFC 1950 §9. miniz_oxide and `gzip -d` reject the
//! stream. Our wrapper re-computes the Adler-32 over the plaintext
//! and replaces the trailer so output is byte-compatible with
//! `gzip`/`zlib`. Filed upstream as `docs/omnizip-proposals/libdeflate-adler32.md`.
//!
//! ## When to use 0x14
//!
//! Today, prefer 0x05 unless you specifically need:
//! - A second independent DEFLATE implementation for differential
//!   testing.
//! - The promise of faster decode on benchmarks where omnizip's
//!   libdeflate beats miniz_oxide (decode-heavy workloads).

use crate::codec::Codec;
use crate::error::CoreError;

/// libdeflate-compatible DEFLATE codec.
pub struct LibdeflateCodec;

impl Codec for LibdeflateCodec {
    fn id(&self) -> u8 {
        super::CODEC_LIBDEFLATE
    }

    fn name(&self) -> &'static str {
        "libdeflate"
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        let codec = omnizip_libdeflate::LibdeflateCodec::new();
        let mut out = omnizip_codecs::Codec::compress(
            &codec,
            plaintext,
            omnizip_codecs::CompressionLevel::default(),
        )
        .map_err(libdeflate_err)?;
        // Workaround for omnizip-libdeflate 0.14.6 Adler-32 bug:
        // recompute over plaintext and patch the trailer. zlib stream
        // layout is [2-byte header][deflate body][4-byte Adler-32 BE].
        if out.len() >= 6 {
            let correct = adler32(plaintext);
            let trailer_start = out.len() - 4;
            out[trailer_start..].copy_from_slice(&correct.to_be_bytes());
        }
        Ok(out)
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
            reason: format!("decompress: expected_len {expected_len} exceeds usize"),
        })?;
        let codec = omnizip_libdeflate::LibdeflateCodec::new();
        let result =
            omnizip_codecs::Codec::decompress(&codec, compressed, expected_len).map_err(libdeflate_err)?;
        if result.len() != expected_us {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "libdeflate decompress: result length {} does not match plaintext_len {expected_us}",
                    result.len()
                ),
            });
        }
        Ok(result)
    }
}

fn libdeflate_err(e: omnizip_codecs::OmnizipError) -> CoreError {
    CoreError::Corrupt {
        reason: format!("libdeflate: {e}"),
    }
}

/// Compute the Adler-32 checksum of `data` per RFC 1950 §9.
fn adler32(data: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + u32::from(byte)) % MOD_ADLER;
        b = (b + a) % MOD_ADLER;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_text() {
        let data = b"The quick brown fox jumps over the lazy dog. ".repeat(50);
        let codec = LibdeflateCodec;
        let compressed = codec.compress(&data).expect("compress");
        let recovered = codec
            .decompress(&compressed, data.len() as u32)
            .expect("decompress");
        assert_eq!(recovered, data);
    }

    #[test]
    fn round_trip_empty() {
        let data: Vec<u8> = Vec::new();
        let codec = LibdeflateCodec;
        let compressed = codec.compress(&data).expect("compress empty");
        let recovered = codec
            .decompress(&compressed, 0)
            .expect("decompress empty");
        assert_eq!(recovered, data);
    }

    #[test]
    fn cross_decodes_with_deflate() {
        // Wire format is RFC 1951 + zlib wrapper for both codecs.
        // A 0x14 (libdeflate) compressed stream must decode through
        // 0x05 (deflate) and vice versa.
        use crate::codec::deflate::DeflateCodec;
        let data = b"cross-decode test data ".repeat(20);

        let libdeflate_compressed = LibdeflateCodec.compress(&data).expect("libdeflate compress");
        let recovered_via_deflate =
            DeflateCodec
                .decompress(&libdeflate_compressed, data.len() as u32)
                .expect("decode libdeflate via deflate");
        assert_eq!(recovered_via_deflate, data);

        let deflate_compressed = DeflateCodec.compress(&data).expect("deflate compress");
        let recovered_via_libdeflate =
            LibdeflateCodec
                .decompress(&deflate_compressed, data.len() as u32)
                .expect("decode deflate via libdeflate");
        assert_eq!(recovered_via_libdeflate, data);
    }

    #[test]
    fn rejects_length_mismatch() {
        let data = b"hello world";
        let compressed = LibdeflateCodec.compress(data).expect("compress");
        match LibdeflateCodec.decompress(&compressed, 99) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(
                    reason.contains("does not match") || reason.contains("mismatch"),
                    "got: {reason}"
                );
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn adler32_known_values() {
        // RFC 1950 §9 examples.
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"a"), 0x00620062);
        assert_eq!(adler32(b"abc"), 0x024d0127);
    }
}
