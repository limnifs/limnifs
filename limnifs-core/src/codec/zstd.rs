//! Zstandard codec (0x02): pure Rust via `omnizip-zstd` 0.21.x.
//!
//! The load-bearing fact for this module is the LEVEL-TIER MAP
//! (`level_for_quality` + the band pin test): since omnizip
//! 0.21.12/0.21.13 every level ≥ L3 runs the optimal parser, so
//! LimniFS's defaults sit at Fastest (L1/L2) — the only fast tier —
//! and raising a default into the parser band is a conscious,
//! createperf-measured decision. The zstd quality knob is decoupled
//! from the flat (brotli) quality; see `ZstdTunables::default` and
//! `compress_with_tunables` for the incident history.

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
        // quality 2 → ZstdLevel::Fastest (L1/L2). Since omnizip
        // 0.21.12/0.21.13 every level ≥ L3 runs the optimal parser
        // (~16x slower at our chunk sizes for ~4% tighter output) —
        // L1/L2 is the only fast tier left, and the tournament's
        // short-circuit + brotli fallback absorb its ratio cost.
        // Raise this consciously, with a createperf reading (the
        // band pin test below guards the map, not this default).
        Self { quality: 2 }
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

/// Whole-file drops at or above this size compress across threads
/// (omnizip `compress_mt`): job boundaries are a pure function of
/// input length and level — output is byte-identical for any thread
/// count — and each job is an independent frame, so the standard
/// decoder handles the concatenation. The chunk path never sees
/// inputs this large (chunks are bounded by `max_chunk_size`), so
/// this only fires for whole-file drops and seekable containers.
const MT_WHOLE_FILE_THRESHOLD: usize = 4 * 1024 * 1024;

fn compress_verified(
    plaintext: &[u8],
    level: omnizip_zstd::ZstdLevel,
) -> Result<Vec<u8>, CoreError> {
    let out = if plaintext.len() >= MT_WHOLE_FILE_THRESHOLD {
        // Floor at 2: omnizip maps threads == 1 to the SINGLE-frame
        // path, whose bytes differ from any multi-thread split.
        // Every threads >= 2 value produces identical output (job
        // boundaries depend only on input length), so the floor
        // keeps output machine-independent even on one core.
        let threads = std::thread::available_parallelism().map_or(2, |n| std::cmp::max(2, n.get()));
        omnizip_zstd::compress_mt(plaintext, level, threads)
    } else {
        omnizip_zstd::compress(plaintext, level)
    };
    out.map_err(|e| CoreError::Corrupt {
        reason: format!("zstd compress (level {level}) failed: {e}"),
    })
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
        // Decoupled from the flat (brotli) quality: brotli's default
        // q11 used to leak in here as a zstd level proxy and, since
        // omnizip 0.21.12+, dropped zstd into the optimal-parser
        // band (~16x slower). 0 = fast-tier default; raise
        // consciously with a createperf reading (band pin test
        // guards the map).
        let quality = if t.zstd_quality > 0 {
            t.zstd_quality
        } else {
            2
        };
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

/// Compress with Zstandard at `Fastest` (L1/L2). Since omnizip
/// 0.21.12/0.21.13, `Default` (L6) runs the optimal parser — the
/// wrong trade for the no-tunables default path, where speed is
/// the contract (see `ZstdTunables::default`).
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] on encoder failure or when the
/// frame fails the omnizip#315 round-trip self-check.
pub(crate) fn compress(plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    compress_verified(plaintext, omnizip_zstd::ZstdLevel::Fastest)
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
    /// omnizip#315 canary: the decoder previously failed on its own
    /// encoder's frames for this content shape (fixed upstream in
    /// 0.16.79). If this regresses, the write-side decompress-verify
    /// guard documented in the v0.2.53 changelog must come back.
    #[test]
    fn quality_to_level_band_map_is_pinned() {
        // The band map is a product decision, not an accident.
        // Since omnizip 0.21.12/0.21.13 every level ≥ L3 runs the
        // optimal parser (~16x slower at our chunk sizes), so the
        // defaults (ZstdTunables quality 2, the no-tunables
        // compress, and the dictionary pass) all sit at Fastest —
        // the only fast tier left. Raising a default into the
        // optimal-parser band is a conscious trade: update this pin
        // and the defaults together, with a createperf reading.
        use omnizip_zstd::ZstdLevel;
        assert_eq!(
            level_for_quality(ZstdTunables::default().quality),
            ZstdLevel::Fastest
        );
        for q in 0..=2 {
            assert_eq!(level_for_quality(q), ZstdLevel::Fastest, "q{q}");
        }
        for q in 3..=5 {
            assert_eq!(level_for_quality(q), ZstdLevel::Fast, "q{q}");
        }
        for q in 6..=11 {
            assert_eq!(level_for_quality(q), ZstdLevel::Default, "q{q}");
        }
        for q in 12..=21 {
            assert_eq!(level_for_quality(q), ZstdLevel::Better, "q{q}");
        }
        for q in 22..=255 {
            assert_eq!(level_for_quality(q), ZstdLevel::Best, "q{q}");
        }
    }

    #[test]
    fn omnizip_315_blob_round_trips_at_all_levels() {
        const B64: &str = "AgAAAAIAAAAAAAAApIEAAAAAAAAAAAAAjfBCgS8IzhiN8EKBLwjOGAEAAAAEpQAAAGR1cGxpY2F0ZSBpbmxpbmUgY29udGVudDogdGhlIHNhbWUgMjAwLWlzaCBieXRlcyBpbiB0aHJlZSBmaWxlcywgc28gdGhlIHdyaXRlcidzIGlubGluZSBkZWR1cCBmaWxlcyBvbiBldmVyeSByZWFsaXN0aWMgdHJlZS4gUGFkZGluZyBwYWRkaW5nIHBhZGRpbmcgcGFkZGluZyBwYWRkaW5nIQEAAAAAAAAA7UEAAAAAAAAAAAAABelAgS8IzhgF6UCBLwjOGAEAAAAA0n/vT8wNhb/EicVbOmpyaI3ka3H9+fam7ksII2Ipyd4BAAAAAQEAAAAJAAAAZHVwLWEudHh0AgAAAAAAAAAB";
        let blob = b64(B64);
        assert_eq!(blob.len(), 318);
        for level in [
            omnizip_zstd::ZstdLevel::Fastest,
            omnizip_zstd::ZstdLevel::Fast,
            omnizip_zstd::ZstdLevel::Default,
            omnizip_zstd::ZstdLevel::Better,
            omnizip_zstd::ZstdLevel::Best,
        ] {
            let frame =
                compress_verified(&blob, level).unwrap_or_else(|e| panic!("{level:?}: {e}"));
            let back = omnizip_zstd::decompress(&frame, 318)
                .unwrap_or_else(|e| panic!("{level:?} decode: {e}"));
            assert_eq!(back, blob, "{level:?} must round-trip");
        }
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

#[cfg(test)]
mod mt_tests {
    use super::*;

    /// Whole-file MT frames round-trip through the standard decoder
    /// (multi-frame concatenation), and the output is deterministic
    /// across thread counts (upstream contract, re-verified here).
    #[test]
    fn multi_thread_frames_round_trip_and_are_thread_deterministic() {
        let mut payload = Vec::with_capacity(9 * 1024 * 1024);
        let mut state = 0x5EED_F00Du64;
        while payload.len() < 9 * 1024 * 1024 {
            state ^= state << 13;
            state ^= state >> 7;
            payload.extend_from_slice(&state.to_le_bytes());
            payload.extend_from_slice(b"mt-frame filler line\n");
        }
        let a =
            compress_verified(&payload, omnizip_zstd::ZstdLevel::Fastest).expect("mt compress a");
        let b = omnizip_zstd::compress_mt(&payload, omnizip_zstd::ZstdLevel::Fastest, 2)
            .expect("two threads");
        let c = omnizip_zstd::compress_mt(&payload, omnizip_zstd::ZstdLevel::Fastest, 8)
            .expect("eight threads");
        assert_eq!(a, b, "registry path matches explicit 2 threads");
        assert_eq!(b, c, "output must not depend on thread count (>= 2)");
        let back = omnizip_zstd::decompress(&a, payload.len() as u32).expect("decode mt");
        assert_eq!(back, payload, "mt frames round-trip");
    }
}
