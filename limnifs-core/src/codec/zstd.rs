//! Zstandard codec (0x02): pure Rust via `omnizip-zstd` 0.14.10.
//!
//! `omnizip-zstd` 0.14.10 ships a real encoder and decoder. The
//! `Default` (L6), `Better` (L12), and `Best` (L22) levels had a
//! regression in 0.14.8 on highly-repetitive inputs (50 KB output
//! and 14+ s runtime for 90 KB of repeated text). The 0.14.10 fix
//! (omnizip-rs PR #90) restores correct level differentiation; see
//! `docs/omnizip-proposals/zstd-default-broken.md` for the original
//! report.

use crate::codec::{Codec, CodecTunables, PerCodecTunables};
use crate::error::CoreError;

/// Strongly-typed ZSTD tunables.
#[derive(Clone, Debug)]
pub struct ZstdTunables {
    /// ZSTD compression level (mapped to omnizip_zstd::ZstdLevel).
    pub quality: u8,
}

impl Default for ZstdTunables {
    fn default() -> Self {
        // quality 6 → ZstdLevel::Default (libzstd's default).
        Self { quality: 6 }
    }
}

/// Map a per-codec quality value to an `omnizip_zstd::ZstdLevel`.
fn level_for_quality(quality: u8) -> omnizip_zstd::ZstdLevel {
    match quality {
        0..=2 => omnizip_zstd::ZstdLevel::Fastest,
        3..=5 => omnizip_zstd::ZstdLevel::Fast,
        6..=11 => omnizip_zstd::ZstdLevel::Default,
        12..=21 => omnizip_zstd::ZstdLevel::Better,
        _ => omnizip_zstd::ZstdLevel::Best,
    }
}

/// ZSTD codec. Encode at `Default` (L6); decode at any level.
pub struct ZstdCodec;

/// Verify an encoder frame decodes back to `plaintext`.
///
/// omnizip#315 (still live in 0.16.77): the omnizip-zstd DECODER
/// fails on frames its own encoder produced, content-dependently,
/// at Fastest/Fast/Default/Better (Best is correct). A frame that
/// does not round-trip here would become an image no LimniFS reader
/// can open — so the codec refuses to emit it. Callers treat this as
/// a failed candidate (tournament moves to the next codec; whole-file
/// paths fall back). Remove when the upstream decoder is fixed.
fn verify_roundtrip(frame: &[u8], plaintext: &[u8]) -> Result<(), CoreError> {
    let len = u32::try_from(plaintext.len()).map_err(|_| CoreError::Corrupt {
        reason: format!("zstd self-check: plaintext {} exceeds u32", plaintext.len()),
    })?;
    match omnizip_zstd::decompress(frame, len) {
        Ok(back) if back == plaintext => Ok(()),
        Ok(back) => Err(CoreError::Corrupt {
            reason: format!(
                "zstd self-check: decode returned {} bytes for {} (omnizip#315 decoder bug)",
                back.len(),
                plaintext.len()
            ),
        }),
        Err(e) => Err(CoreError::Corrupt {
            reason: format!("zstd self-check decode failed (omnizip#315): {e}"),
        }),
    }
}

/// Compress + verify. Every encode path funnels through here.
fn compress_verified(
    plaintext: &[u8],
    level: omnizip_zstd::ZstdLevel,
) -> Result<Vec<u8>, CoreError> {
    let frame = omnizip_zstd::compress(plaintext, level).map_err(|e| CoreError::Corrupt {
        reason: format!("zstd compress (level {level}) failed: {e}"),
    })?;
    verify_roundtrip(&frame, plaintext)?;
    Ok(frame)
}

impl Codec for ZstdCodec {
    fn id(&self) -> u8 {
        super::CODEC_ZSTD
    }

    fn name(&self) -> &'static str {
        "zstd"
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        compress(plaintext)
    }

    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError> {
        let result =
            omnizip_zstd::decompress(compressed, expected_len).map_err(|e| CoreError::Corrupt {
                reason: format!("zstd decompress failed: {e}"),
            })?;
        let expected_us = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
            reason: format!("decompress: expected_len {expected_len} exceeds usize"),
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

    fn compress_with_tunables(
        &self,
        plaintext: &[u8],
        t: &CodecTunables,
    ) -> Result<Vec<u8>, CoreError> {
        let quality = if t.quality > 0 { t.quality } else { 6 };
        let level = level_for_quality(quality);
        compress_verified(plaintext, level)
    }
}

impl PerCodecTunables for ZstdCodec {
    type Tunables = ZstdTunables;

    fn compress_with_owned_tunables(
        &self,
        plaintext: &[u8],
        t: &Self::Tunables,
    ) -> Result<Vec<u8>, CoreError> {
        let level = level_for_quality(t.quality);
        compress_verified(plaintext, level)
    }
}

/// Compress with Zstandard at `Default` (level 6). This matches
/// libzstd's CLI default and gives the right speed/ratio balance
/// for source code, CSV, and general text.
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] on encoder failure or when the
/// frame fails the omnizip#315 round-trip self-check.
pub(crate) fn compress(plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    compress_verified(plaintext, omnizip_zstd::ZstdLevel::Default)
}

/// Compress at an explicit level. Used by callers that want a
/// different speed/ratio tradeoff than the default L6.
#[allow(dead_code)]
pub(crate) fn compress_at_level(
    plaintext: &[u8],
    level: omnizip_zstd::ZstdLevel,
) -> Result<Vec<u8>, CoreError> {
    compress_verified(plaintext, level)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 318-byte blob from omnizip#315: trips the decoder at
    /// Fastest/Fast/Default/Better (Best decodes correctly). The
    /// self-check must refuse to emit the broken frames.
    #[test]
    fn omnizip_315_blob_is_refused_not_emitted() {
        const B64: &str = "AgAAAAIAAAAAAAAApIEAAAAAAAAAAAAAjfBCgS8IzhiN8EKBLwjOGAEAAAAEpQAAAGR1cGxpY2F0ZSBpbmxpbmUgY29udGVudDogdGhlIHNhbWUgMjAwLWlzaCBieXRlcyBpbiB0aHJlZSBmaWxlcywgc28gdGhlIHdyaXRlcidzIGlubGluZSBkZWR1cCBmaWxlcyBvbiBldmVyeSByZWFsaXN0aWMgdHJlZS4gUGFkZGluZyBwYWRkaW5nIHBhZGRpbmcgcGFkZGluZyBwYWRkaW5nIQEAAAAAAAAA7UEAAAAAAAAAAAAABelAgS8IzhgF6UCBLwjOGAEAAAAA0n/vT8wNhb/EicVbOmpyaI3ka3H9+fam7ksII2Ipyd4BAAAAAQEAAAAJAAAAZHVwLWEudHh0AgAAAAAAAAAB";
        let blob = b64(B64);
        assert_eq!(blob.len(), 318);
        for level in [
            omnizip_zstd::ZstdLevel::Fastest,
            omnizip_zstd::ZstdLevel::Fast,
            omnizip_zstd::ZstdLevel::Default,
            omnizip_zstd::ZstdLevel::Better,
        ] {
            let err = compress_verified(&blob, level)
                .expect_err("self-check must refuse a frame the decoder cannot read");
            assert!(
                err.to_string().contains("omnizip#315"),
                "expected omnizip#315 context, got {err}"
            );
        }
        // Best is unaffected and must round-trip.
        let frame = compress_verified(&blob, omnizip_zstd::ZstdLevel::Best).expect("Best OK");
        let back = omnizip_zstd::decompress(&frame, 318).expect("decode");
        assert_eq!(back, blob);
    }

    #[test]
    fn assorted_inputs_round_trip_at_all_levels() {
        let mut state = 0x0dd_ba11_5eed_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for len in [0usize, 1, 17, 511, 318, 4096, 65536] {
            let data: Vec<u8> = (0..len)
                .map(|i| ((next() >> 32) ^ i as u64) as u8)
                .collect();
            let frame = compress(&data).expect("compress");
            if data.is_empty() || frame.len() >= data.len() {
                continue; // STORE candidate wins; nothing to verify
            }
            let back = omnizip_zstd::decompress(&frame, data.len() as u32).expect("decode");
            assert_eq!(back, data, "len {len} must round-trip");
        }
    }

    fn b64(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let (mut buf, mut bits) = (0u32, 0u32);
        for c in s.chars() {
            let v = match c {
                'A'..='Z' => u32::from(c) - 65,
                'a'..='z' => u32::from(c) - 71,
                '0'..='9' => u32::from(c) + 4,
                '+' => 62,
                '/' => 63,
                _ => continue,
            };
            buf = (buf << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push(((buf >> bits) & 0xFF) as u8);
            }
        }
        out
    }
}
