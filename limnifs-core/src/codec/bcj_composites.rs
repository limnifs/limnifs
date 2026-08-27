//! BCJ (Branch / Call / Jump) composite codecs (ids 0x20..=0x2F).
//!
//! Each composite codec applies a BCJ filter (x86 / ARM64 / etc.) to
//! convert relative call/branch addresses in executable code to
//! absolute values, then compresses the filtered bytes with an inner
//! codec (LZ4 / ZSTD). Decompression is the exact inverse.
//!
//! ## Wire format
//!
//! ```text
//! [filtered_len: u32 LE][inner_codec_bytes...]
//! ```
//!
//! The 4-byte length prefix lets the inner codec's decoder validate
//! its output length without knowing the original plaintext length
//! (the BCJ filter is byte-preserving but not length-changing).
//!
//! ## Why this is DRY
//!
//! All composites share the same shape: `filter.encode → inner.compress
//! → length-prefix`. The [`filter_then_compress`] and
//! [`decompress_then_filter`] helper functions contain the entire
//! pipeline; each `Codec` impl is 10 lines of glue.
//!
//! ## Algorithm sources
//!
//! - x86 BCJ: derived from the LZMA SDK's BCJ x86 filter description
//!   (Igor Pavlov, public domain).
//! - ARM64 BCJ: derived from the same SDK's ARM64 filter.
//!
//! See `TODO.impl/04-writer-pipeline/04-bcj-composite-codecs.md` for
//! the full design and acceptance.

use omnizip_filters::Filter;

use crate::codec::composite;
use crate::codec::{Codec, CodecTunables, CoreError, CODEC_LZ4, CODEC_ZSTD};

/// Codec id 0x20: BCJ-x86 filter + LZ4.
pub const CODEC_BCJ_X86_LZ4: u8 = 0x20;
/// Codec id 0x21: BCJ-x86 filter + ZSTD.
pub const CODEC_BCJ_X86_ZSTD: u8 = 0x21;
/// Codec id 0x23: BCJ-ARM64 filter + LZ4.
pub const CODEC_BCJ_ARM64_LZ4: u8 = 0x23;
/// Codec id 0x24: BCJ-ARM64 filter + ZSTD.
pub const CODEC_BCJ_ARM64_ZSTD: u8 = 0x24;

/// Minimum input size worth running through BCJ. The filter has no
/// effect on tiny inputs (no calls to convert) and adds overhead.
const MIN_BCJ_SIZE: usize = 1024;

/// BCJ-x86 filter + LZ4 composite (id 0x20) as an instance of
/// [`FilterCodecComposite`](crate::codec::composite::FilterCodecComposite).
pub type BcjX86Lz4Codec = composite::FilterCodecComposite<omnizip_filters::BcjX86Filter>;
/// BCJ-x86 filter + ZSTD composite (id 0x21).
pub type BcjX86ZstdCodec = composite::FilterCodecComposite<omnizip_filters::BcjX86Filter>;
/// BCJ-ARM64 filter + LZ4 composite (id 0x23).
pub type BcjArm64Lz4Codec = composite::FilterCodecComposite<omnizip_filters::BcjArm64Filter>;
/// BCJ-ARM64 filter + ZSTD composite (id 0x24).
pub type BcjArm64ZstdCodec = composite::FilterCodecComposite<omnizip_filters::BcjArm64Filter>;

/// Minimum input size worth running through BCJ: the filter has no
/// effect on tiny inputs (no relative calls to convert) and adds
/// overhead.

/// BCJ-x86 + LZ4.
#[must_use]
pub fn bcj_x86_lz4() -> BcjX86Lz4Codec {
    BcjX86Lz4Codec::new(
        omnizip_filters::BcjX86Filter,
        crate::codec::CODEC_LZ4,
        CODEC_BCJ_X86_LZ4,
        "bcj-x86-lz4",
        MIN_BCJ_SIZE,
    )
}

/// BCJ-x86 + ZSTD.
#[must_use]
pub fn bcj_x86_zstd() -> BcjX86ZstdCodec {
    BcjX86ZstdCodec::new(
        omnizip_filters::BcjX86Filter,
        crate::codec::CODEC_ZSTD,
        CODEC_BCJ_X86_ZSTD,
        "bcj-x86-zstd",
        MIN_BCJ_SIZE,
    )
}

/// BCJ-ARM64 + LZ4.
#[must_use]
pub fn bcj_arm64_lz4() -> BcjArm64Lz4Codec {
    BcjArm64Lz4Codec::new(
        omnizip_filters::BcjArm64Filter,
        crate::codec::CODEC_LZ4,
        CODEC_BCJ_ARM64_LZ4,
        "bcj-arm64-lz4",
        MIN_BCJ_SIZE,
    )
}

/// BCJ-ARM64 + ZSTD.
#[must_use]
pub fn bcj_arm64_zstd() -> BcjArm64ZstdCodec {
    BcjArm64ZstdCodec::new(
        omnizip_filters::BcjArm64Filter,
        crate::codec::CODEC_ZSTD,
        CODEC_BCJ_ARM64_ZSTD,
        "bcj-arm64-zstd",
        MIN_BCJ_SIZE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Codec;

    /// Build a synthetic "executable-like" buffer: many x86
    /// relative CALL/JMP instructions targeting a small set of
    /// addresses. After BCJ-x86 the relative offsets become
    /// absolute, exposing massive redundancy LZ4 can dedup.
    fn synthetic_x86_calls(size_bytes: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(size_bytes);
        // Targets (absolute addresses we want calls to land on).
        let targets: [u32; 4] = [0x401000, 0x401234, 0x402000, 0x405060];
        let mut i = 0u32;
        while out.len() + 5 <= size_bytes {
            // 0xE8 = CALL rel32; 0xE9 = JMP rel32.
            let opcode = if i & 1 == 0 { 0xE8 } else { 0xE9 };
            let target = targets[(i as usize) & 3];
            // The relative offset is target - (out.len() + 5).
            let here = out.len() as u32 + 5;
            let rel: i32 = (target as i32) - (here as i32);
            out.push(opcode);
            out.extend_from_slice(&rel.to_le_bytes());
            i += 1;
        }
        out
    }

    #[test]
    fn bcj_x86_lz4_round_trips() {
        let input = synthetic_x86_calls(64 * 1024);
        let codec = bcj_x86_lz4();
        let compressed = codec.compress(&input).expect("compress");
        let recovered = codec
            .decompress(&compressed, input.len() as u32)
            .expect("decompress");
        assert_eq!(recovered, input);
    }

    #[test]
    fn bcj_x86_lz4_beats_plain_lz4_on_synthetic_exec() {
        let input = synthetic_x86_calls(64 * 1024);
        let plain = crate::codec::compress(CODEC_LZ4, &input).expect("plain lz4");
        let bcj = bcj_x86_lz4().compress(&input).expect("bcj+lz4");
        assert!(
            bcj.len() < plain.len(),
            "BCJ+LZ4 ({}) should beat plain LZ4 ({}) on synthetic x86 calls",
            bcj.len(),
            plain.len()
        );
    }

    #[test]
    fn bcj_x86_zstd_round_trips() {
        let input = synthetic_x86_calls(32 * 1024);
        let codec = bcj_x86_zstd();
        let compressed = codec.compress(&input).expect("compress");
        let recovered = codec
            .decompress(&compressed, input.len() as u32)
            .expect("decompress");
        assert_eq!(recovered, input);
    }

    #[test]
    fn bcj_arm64_lz4_round_trips() {
        // ARM64 BCJ doesn't help synthetic x86 input — the filter is
        // architecture-specific — but it must still round-trip
        // (filter is its own inverse, regardless of input semantics).
        let input = synthetic_x86_calls(32 * 1024);
        let codec = bcj_arm64_lz4();
        let compressed = codec.compress(&input).expect("compress");
        let recovered = codec
            .decompress(&compressed, input.len() as u32)
            .expect("decompress");
        assert_eq!(recovered, input);
    }

    #[test]
    fn bcj_x86_lz4_handles_tiny_input() {
        // Inputs below MIN_BCJ_SIZE are still valid; they just don't
        // benefit. Round-trip must succeed.
        let input = b"hello world";
        let codec = bcj_x86_lz4();
        let compressed = codec.compress(input).expect("compress");
        let recovered = codec
            .decompress(&compressed, input.len() as u32)
            .expect("decompress");
        assert_eq!(recovered.as_slice(), &input[..]);
    }
}
