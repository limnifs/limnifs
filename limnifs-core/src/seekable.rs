//! Seekable drop container (slab format v2, TODO.sota-fs/05).
//!
//! Prior art: the zstd Seekable Format — independent frames plus a
//! trailing index — and Fuchsia BlobFS seek-table reads. A drop whose
//! plaintext is large cannot serve a cold random read without
//! decompressing the whole payload: tebako measured ~48 GiB of wasted
//! decode reading a 19.5 MiB file through 8 KiB windows (limnifs#192).
//!
//! The container is codec-agnostic: every frame is an independently
//! decodable stream of one codec covering one contiguous plaintext
//! sub-range, and a footer indexes them.
//!
//! ```text
//! container := frame* footer
//! frame      := independently-decodable codec stream covering one
//!               contiguous uncompressed sub-range
//! footer     := per frame (order): u32 uncomp_len, u32 comp_len,
//!               then a fixed 12-byte tail:
//!               magic "LMSK", u16 version=1, u32 frame_count
//! ```
//!
//! `plaintext_range` decompresses only the frames covering the
//! requested window (binary search on cumulative uncompressed
//! offsets), so a cold 8 KiB read on a 19.5 MiB drop costs at most
//! one 256 KiB frame.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::codec::{self, CodecTunables};
use crate::error::CoreError;

/// Process-wide count of container frames decompressed. Windowed-read
/// canaries (TODO.sota-fs/06) assert cold random reads touch a
/// bounded number of frames instead of the whole drop.
static FRAMES_DECODED: AtomicU64 = AtomicU64::new(0);

/// Frames decompressed since process start (see [`FRAMES_DECODED`]).
#[must_use]
pub fn frames_decoded() -> u64 {
    FRAMES_DECODED.load(Ordering::Relaxed)
}

/// Count one frame decode performed outside this module (the
/// frame-cached range path in `slab_cache` decodes frames directly).
pub(crate) fn count_frame_decode() {
    FRAMES_DECODED.fetch_add(1, Ordering::Relaxed);
}

/// Cache key for one frame of one drop: BLAKE3(drop_id ‖ frame_index).
#[must_use]
pub fn frame_key(drop_id: &[u8; 32], frame_index: u32) -> [u8; 32] {
    let mut buf = [0u8; 36];
    buf[..32].copy_from_slice(drop_id);
    buf[32..36].copy_from_slice(&frame_index.to_le_bytes());
    crate::merkle::hash_section(&buf)
}

/// Magic trailing every seekable container footer.
pub const SEEKABLE_MAGIC: [u8; 4] = *b"LMSK";

/// Container layout version.
pub const SEEKABLE_VERSION: u16 = 1;

/// Fixed footer tail: magic (4) + version (2) + frame_count (4).
/// The count lives in the tail (fixed distance from the container's
/// end) so the footer can be parsed back-to-front without knowing
/// the frame count up front — same trick as zstd's seekable footer.
const FOOTER_TAIL_LEN: usize = 10;

/// Target uncompressed frame size. Matches the zstd seekable format's
/// recommended frame size: large enough that per-frame codec overhead
/// is amortised, small enough that a cold random read stays cheap.
pub const SEEKABLE_FRAME_SIZE: usize = 256 * 1024;

/// Writer emission threshold: drops whose plaintext exceeds this
/// length are encoded as containers (when their codec is
/// [`is_seekable_codec`]).
pub const SEEKABLE_EMISSION_THRESHOLD: usize = 1024 * 1024;

/// Drop-record flag: the window bytes are a seekable container.
pub const DROP_FLAG_SEEKABLE: u8 = 0x01;

/// Can this codec's output be split into independent frames?
///
/// General stream codecs (LZ4, ZSTD, XZ, Brotli, Deflate, Snappy,
/// PPMd, bzip2, ...) encode any sub-range standalone, so a container
/// of them is seekable. Excluded:
/// - STORE — already trivially seekable, frames would add overhead;
/// - FLAC / RICEPP / GLZA — whole-stream models (audio/entropy
///   coders that need the complete input);
/// - trained-dictionary and filter composites — their win comes from
///   cross-frame shared state (BCJ filters carry instruction-window
///   state; dict training spans the whole file).
#[must_use]
pub fn is_seekable_codec(codec_id: u8) -> bool {
    matches!(
        codec_id,
        codec::CODEC_LZ4
            | codec::CODEC_LZ4_HC
            | codec::CODEC_ZSTD
            | codec::CODEC_XZ
            | codec::CODEC_BROTLI
            | codec::CODEC_DEFLATE
            | codec::CODEC_SNAPPY
            | codec::CODEC_BZIP2
            | codec::CODEC_PPMD
            | codec::CODEC_PPMD8
            | codec::CODEC_LIBDEFLATE
            | codec::CODEC_DEFLATE64
    )
}

/// Encode `plaintext` as a seekable container of `codec_id` frames.
///
/// Frames are fixed at [`SEEKABLE_FRAME_SIZE`] uncompressed bytes
/// (the last frame may be shorter); the drop's `DropId` still hashes
/// the full plaintext, so dedup is unaffected.
///
/// # Errors
///
/// Propagates codec compression errors.
pub fn encode_seekable(
    codec_id: u8,
    plaintext: &[u8],
    tunables: &CodecTunables,
) -> Result<Vec<u8>, CoreError> {
    let chunks: Vec<&[u8]> = if plaintext.is_empty() {
        vec![&[][..]]
    } else {
        plaintext.chunks(SEEKABLE_FRAME_SIZE).collect()
    };
    let frame_count = chunks.len();
    let mut body = Vec::with_capacity(plaintext.len() / 2 + frame_count * 8);
    let mut comp_lens = Vec::with_capacity(frame_count);
    for frame in &chunks {
        let compressed = codec::compress_with_tunables(codec_id, frame, tunables)?;
        comp_lens.push(
            u32::try_from(compressed.len()).map_err(|_| CoreError::Corrupt {
                reason: "seekable frame compressed length exceeds u32".into(),
            })?,
        );
        body.extend_from_slice(&compressed);
    }
    let mut out = body;
    for (frame, comp_len) in chunks.iter().zip(&comp_lens) {
        out.extend_from_slice(
            &u32::try_from(frame.len())
                .expect("frame plaintext fits u32")
                .to_le_bytes(),
        );
        out.extend_from_slice(&comp_len.to_le_bytes());
    }
    out.extend_from_slice(&SEEKABLE_MAGIC);
    out.extend_from_slice(&SEEKABLE_VERSION.to_le_bytes());
    out.extend_from_slice(
        &u32::try_from(frame_count)
            .expect("frame count fits u32")
            .to_le_bytes(),
    );
    Ok(out)
}

/// Parsed container footer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeekFooter {
    /// Uncompressed length of each frame, in order.
    pub uncomp_lens: Vec<u32>,
    /// Compressed length of each frame, in order.
    pub comp_lens: Vec<u32>,
    /// Cumulative uncompressed offset at the START of frame `i`
    /// (`starts[i]`), plus the total as the final entry. Precomputed
    /// once at parse time so the hot windowed-read path (footer is
    /// memoized per drop) pays no per-window allocation or scan to
    /// locate the covering frame.
    starts: Vec<u64>,
    /// Cached `starts[len]` — the drop's plaintext length.
    total_uncomp: u64,
}

impl SeekFooter {
    /// Cumulative uncompressed offset at the START of frame `i`.
    #[must_use]
    pub fn uncomp_offset(&self, i: usize) -> u64 {
        self.starts[i.min(self.starts.len() - 1)]
    }

    /// Total uncompressed length.
    #[must_use]
    pub const fn total_uncomp(&self) -> u64 {
        self.total_uncomp
    }

    /// Total compressed length of all frames (footer excluded).
    #[must_use]
    pub fn total_comp(&self) -> u64 {
        self.comp_lens.iter().map(|&l| u64::from(l)).sum()
    }

    /// Index of the first frame whose plaintext range crosses `off`
    /// (the frame containing `off`). O(log n) over the precomputed
    /// cumulative starts — no allocation.
    #[must_use]
    pub fn frame_containing(&self, off: u64) -> usize {
        self.starts.partition_point(|&s| s <= off).saturating_sub(1)
    }

    /// Cumulative compressed offset of frame `i`'s bytes in the
    /// container (sum of the compressed lengths before it).
    #[must_use]
    pub fn compressed_offset_of(&self, i: usize) -> usize {
        self.comp_lens[..i].iter().map(|&l| l as usize).sum()
    }
}

/// Parse the footer at the end of `container`.
///
/// # Errors
///
/// - [`CoreError::Corrupt`] when the magic/version is wrong, the
///   lengths overflow, or the footer size is inconsistent with the
///   container length.
pub(crate) fn parse_footer(container: &[u8]) -> Result<SeekFooter, CoreError> {
    let corrupt = |reason: String| CoreError::Corrupt {
        reason: format!("seekable container: {reason}"),
    };
    if container.len() < 8 + FOOTER_TAIL_LEN {
        return Err(corrupt(format!(
            "length {} too small for one frame entry + footer",
            container.len()
        )));
    }
    let tail = &container[container.len() - FOOTER_TAIL_LEN..];
    if tail[..4] != SEEKABLE_MAGIC {
        return Err(corrupt("footer magic is not LMSK".into()));
    }
    let version = u16::from_le_bytes([tail[4], tail[5]]);
    if version != SEEKABLE_VERSION {
        return Err(corrupt(format!(
            "footer version {version} (supported: {SEEKABLE_VERSION})"
        )));
    }
    let frame_count = u32::from_le_bytes([tail[6], tail[7], tail[8], tail[9]]) as usize;
    let Some(table_start) = (container.len() - FOOTER_TAIL_LEN).checked_sub(frame_count * 8) else {
        return Err(corrupt(format!(
            "frame_count {frame_count} overruns container length {}",
            container.len()
        )));
    };
    let mut uncomp_lens = Vec::with_capacity(frame_count);
    let mut comp_lens = Vec::with_capacity(frame_count);
    for i in 0..frame_count {
        let e = table_start + i * 8;
        uncomp_lens.push(u32::from_le_bytes([
            container[e],
            container[e + 1],
            container[e + 2],
            container[e + 3],
        ]));
        comp_lens.push(u32::from_le_bytes([
            container[e + 4],
            container[e + 5],
            container[e + 6],
            container[e + 7],
        ]));
    }
    let mut starts = Vec::with_capacity(uncomp_lens.len() + 1);
    let mut acc = 0u64;
    for &l in &uncomp_lens {
        starts.push(acc);
        acc += u64::from(l);
    }
    starts.push(acc);
    let total_uncomp = acc;
    let footer = SeekFooter {
        uncomp_lens,
        comp_lens,
        starts,
        total_uncomp,
    };
    let expected_len = frame_count * 8 + FOOTER_TAIL_LEN;
    let body_len = table_start;
    if footer.total_comp() != body_len as u64 {
        return Err(corrupt(format!(
            "frame lengths sum to {} compressed bytes but container has {}",
            footer.total_comp(),
            body_len
        )));
    }
    if footer.uncomp_lens.iter().any(|&l| l == 0) && footer.uncomp_lens.len() > 1 {
        return Err(corrupt("zero-length frame in multi-frame container".into()));
    }
    Ok(footer)
}

/// Decompress the whole container.
///
/// # Errors
///
/// - [`CoreError::Corrupt`] on footer inconsistency (including
///   `expected_len` mismatch) or frame decode failure.
/// - [`CoreError::UnsupportedFeature`] for unknown codecs.
pub fn decode_seekable(
    codec_id: u8,
    container: &[u8],
    expected_len: u32,
) -> Result<Vec<u8>, CoreError> {
    let footer = parse_footer(container)?;
    if footer.total_uncomp() != u64::from(expected_len) {
        return Err(CoreError::Corrupt {
            reason: format!(
                "seekable container: frames cover {} plaintext bytes, drop record says {expected_len}",
                footer.total_uncomp()
            ),
        });
    }
    let mut out = Vec::with_capacity(expected_len as usize);
    let mut pos = 0usize;
    for (i, &comp_len) in footer.comp_lens.iter().enumerate() {
        let frame = &container[pos..pos + comp_len as usize];
        FRAMES_DECODED.fetch_add(1, Ordering::Relaxed);
        out.extend_from_slice(&codec::decompress(codec_id, frame, footer.uncomp_lens[i])?);
        pos += comp_len as usize;
    }
    Ok(out)
}

/// Decompress only the frames covering `[off, off+len)` of the
/// plaintext. Edge frames are decoded whole and sliced, so the cost
/// of any windowed read is bounded by the frames it touches — a cold
/// 8 KiB read decompresses at most one 256 KiB frame.
///
/// `off + len` must not exceed the drop's plaintext length (the
/// caller clamps via the slice map).
///
/// # Errors
///
/// Inherits [`decode_seekable`] errors.
pub fn decode_seekable_range(
    codec_id: u8,
    container: &[u8],
    off: u64,
    len: usize,
) -> Result<Vec<u8>, CoreError> {
    let footer = parse_footer(container)?;
    let total = footer.total_uncomp();
    if off > total || off + len as u64 > total {
        return Err(CoreError::Corrupt {
            reason: format!(
                "seekable range [{off}, {}) outside plaintext length {total}",
                off + len as u64
            ),
        });
    }
    let first = footer.frame_containing(off);

    let mut out = Vec::with_capacity(len);
    let mut comp_pos = footer.compressed_offset_of(first);
    let mut cum = footer.uncomp_offset(first);
    for i in first..footer.uncomp_lens.len() {
        let uncomp_len = footer.uncomp_lens[i];
        let comp_len = footer.comp_lens[i] as usize;
        let frame = &container[comp_pos..comp_pos + comp_len];
        FRAMES_DECODED.fetch_add(1, Ordering::Relaxed);
        let decoded = codec::decompress(codec_id, frame, uncomp_len)?;
        let slice_from = off.saturating_sub(cum) as usize;
        let slice_to = ((off + len as u64) - cum).min(u64::from(uncomp_len)) as usize;
        out.extend_from_slice(&decoded[slice_from..slice_to]);
        if cum + u64::from(uncomp_len) >= off + len as u64 {
            break;
        }
        comp_pos += comp_len;
        cum += u64::from(uncomp_len);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tunables() -> CodecTunables {
        CodecTunables::default()
    }

    fn payload(len: usize) -> Vec<u8> {
        let mut state = 0x0123_4567_89AB_CDEFu64;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 56) as u8
            })
            .collect()
    }

    #[test]
    fn round_trip_full() {
        let pt = payload(700 * 1024); // 3 frames at 256 KiB
        let container = encode_seekable(codec::CODEC_LZ4, &pt, &tunables()).expect("encode");
        let decoded =
            decode_seekable(codec::CODEC_LZ4, &container, pt.len() as u32).expect("decode");
        assert_eq!(decoded, pt);
        let footer = parse_footer(&container).expect("footer");
        assert_eq!(footer.uncomp_lens.len(), 3);
        assert_eq!(footer.uncomp_lens[0] as usize, SEEKABLE_FRAME_SIZE);
    }

    #[test]
    fn range_reads_match_payload() {
        let pt = payload(600 * 1024);
        let container = encode_seekable(codec::CODEC_ZSTD, &pt, &tunables()).expect("encode");
        // Cold single-frame window, cross-frame window, edge windows.
        for (off, len) in [
            (0usize, 8usize),
            (100 * 1024, 4096),
            (250 * 1024, 300 * 1024), // spans frames 0..2
            (599 * 1024, 1024),
        ] {
            let got = decode_seekable_range(codec::CODEC_ZSTD, &container, off as u64, len)
                .expect("range");
            assert_eq!(got, pt[off..off + len], "off={off} len={len}");
        }
    }

    #[test]
    fn bad_magic_rejected() {
        let pt = payload(10);
        let mut container = encode_seekable(codec::CODEC_LZ4, &pt, &tunables()).expect("encode");
        let last = container.len() - 1;
        container[last] ^= 0xFF;
        assert!(decode_seekable(codec::CODEC_LZ4, &container, 10).is_err());
    }

    #[test]
    fn length_mismatch_rejected() {
        let pt = payload(10);
        let container = encode_seekable(codec::CODEC_LZ4, &pt, &tunables()).expect("encode");
        assert!(decode_seekable(codec::CODEC_LZ4, &container, 11).is_err());
    }

    #[test]
    fn seekable_codec_classification() {
        assert!(is_seekable_codec(codec::CODEC_LZ4));
        assert!(is_seekable_codec(codec::CODEC_ZSTD));
        assert!(is_seekable_codec(codec::CODEC_BROTLI));
        assert!(!is_seekable_codec(codec::CODEC_STORE));
        assert!(!is_seekable_codec(codec::CODEC_FLAC));
        assert!(!is_seekable_codec(codec::CODEC_RICEPP));
        assert!(!is_seekable_codec(codec::CODEC_FSST_BROTLI));
    }
}
