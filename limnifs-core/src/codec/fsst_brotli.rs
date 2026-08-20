//! FSST + Brotli composite codec (id 0x09).
//!
//! FSST (Fast Static Symbol Table) is a preprocessor that finds the
//! most common substrings in a block and replaces each with a single
//! byte. Brotli then compresses the FSST output. The composition
//! exploits substring redundancy that Brotli alone misses at high
//! compression levels (Brotli's window is bounded; FSST's dictionary
//! is built per-block).
//!
//! ## Wire format
//!
//! ```text
//! [u32 LE fsst_compressed_len][fsst_compressed_bytes][brotli_compressed_bytes]
//! ```
//!
//! The FSST section carries its own symbol table; the Brotli section
//! is a standard Brotli stream of the FSST-escaped text. Reader
//! reverses: Brotli decompress → FSST expand.
//!
//! ## When to use
//!
//! CSV/JSON/TSV with strong column-header and value-pattern
//! redundancy. Plain text and source code do not benefit — Brotli
//! alone is already optimal there. The `csv_text` categorizer gates
//! this codec behind a content-sniffing heuristic.

use crate::codec::brotli::DEFAULT_QUALITY;
use crate::codec::CODEC_FSST_BROTLI;
use crate::codec::{brotli, Codec};
use crate::error::CoreError;

/// Codec 0x09 — FSST preprocessor + Brotli.
pub struct FsstBrotliCodec;

impl Codec for FsstBrotliCodec {
    fn id(&self) -> u8 {
        CODEC_FSST_BROTLI
    }
    fn name(&self) -> &'static str {
        "fsst+brotli"
    }

    fn min_compress_size(&self) -> usize {
        256
    }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        compress_with_baseline(plaintext, None)
    }

    fn decompress(&self, compressed: &[u8], _expected_len: u32) -> Result<Vec<u8>, CoreError> {
        if compressed.len() < 4 {
            return Err(CoreError::Corrupt {
                reason: "fsst+brotli: truncated header".into(),
            });
        }
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&compressed[..4]);
        let fsst_len = u32::from_le_bytes(len_bytes) as usize;
        if fsst_len == 0 {
            // No-FSST form: the rest is plain Brotli.
            let brotli_bytes = &compressed[4..];
            return brotli::decompress_at_quality(brotli_bytes, _expected_len);
        }
        if 4 + fsst_len > compressed.len() {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "fsst+brotli: fsst_len {fsst_len} overruns buffer {}",
                    compressed.len()
                ),
            });
        }
        let _fsst_bytes = &compressed[4..4 + fsst_len];
        let brotli_bytes = &compressed[4 + fsst_len..];

        // Decompress Brotli first, then FSST-expand.
        // We don't know the FSST-escaped length ahead of time; pass
        // u32::MAX to skip the length check (Brotli stops at stream end).
        let fsst_escaped = brotli::decompress_at_quality(brotli_bytes, u32::MAX)?;
        let plaintext = omnizip_fsst::decompress(&fsst_escaped).map_err(fsst_err)?;
        Ok(plaintext)
    }
}

/// Pack a plain-Brotli result with a zero-length FSST prefix so the
/// reader knows to skip the FSST stage.
fn pack_no_fsst(brotli_bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + brotli_bytes.len());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(brotli_bytes);
    out
}

fn fsst_err(e: omnizip_codecs::OmnizipError) -> CoreError {
    CoreError::Corrupt {
        reason: format!("fsst: {e}"),
    }
}

/// Compress with an optional pre-computed Brotli baseline.
///
/// Callers that already ran Brotli on `plaintext` (e.g.
/// `process_whole_file_drop`, which compresses every categorizer-routed
/// file with Brotli first) can pass `Some(brotli_c)` to skip the
/// redundant Brotli pass that FSST+Brotli would otherwise run for
/// comparison. The baseline is used as-is — no recompression.
///
/// When `baseline` is `None`, the function runs Brotli on `plaintext`
/// internally (matching the v0.1 behaviour).
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if FSST or Brotli encoding fails.
pub fn compress_with_baseline(
    plaintext: &[u8],
    baseline: Option<&[u8]>,
) -> Result<Vec<u8>, CoreError> {
    // Resolve the plain-Brotli baseline: use the caller-provided bytes
    // if present, else compute it inline.
    let owned_baseline: Option<Vec<u8>>;
    let plain_brotli: &[u8] = match baseline {
        Some(b) => b,
        None => {
            let c = crate::codec::codec_call(|| brotli::compress(plaintext, DEFAULT_QUALITY))?;
            owned_baseline = Some(c);
            owned_baseline.as_deref().unwrap_or_default()
        }
    };

    // Skip FSST for tiny inputs — dictionary overhead exceeds gain.
    if plaintext.len() < 1024 {
        return Ok(pack_no_fsst(plain_brotli));
    }

    // Try FSST + Brotli. If it doesn't beat the plain baseline, fall back.
    let fsst_compressed =
        crate::codec::codec_call(|| omnizip_fsst::compress(plaintext).map_err(fsst_err))?;
    let brotli_input = &fsst_compressed[..];
    let brotli_compressed =
        crate::codec::codec_call(|| brotli::compress(brotli_input, DEFAULT_QUALITY))?;

    let composite_len = 4 + brotli_compressed.len() + fsst_compressed.len();
    if composite_len >= plain_brotli.len() {
        // Plain Brotli wins; emit the no-FSST form. Clone the baseline
        // bytes so we own the output regardless of caller lifetime.
        return Ok(pack_no_fsst(plain_brotli));
    }

    let mut out = Vec::with_capacity(composite_len);
    let fsst_len = u32::try_from(fsst_compressed.len()).map_err(|_| CoreError::Corrupt {
        reason: format!(
            "fsst+brotli: fsst_compressed length {} exceeds u32",
            fsst_compressed.len()
        ),
    })?;
    out.extend_from_slice(&fsst_len.to_le_bytes());
    out.extend_from_slice(&fsst_compressed);
    out.extend_from_slice(&brotli_compressed);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_csv_like_input() {
        let input = b"id,name,city\n1,alice,paris\n2,bob,london\n3,carol,paris\n".repeat(200);
        let c = FsstBrotliCodec;
        let compressed = c.compress(&input).expect("compress");
        // Composite must beat plain Brotli on this highly-redundant input
        // or at least match it (heuristic falls back to plain Brotli).
        let plain = brotli::compress(&input, DEFAULT_QUALITY).expect("plain brotli");
        assert!(
            compressed.len() <= plain.len() + 8,
            "composite ({}) should not be much worse than plain Brotli ({})",
            compressed.len(),
            plain.len()
        );
        let recovered = c
            .decompress(&compressed, input.len() as u32)
            .expect("decompress");
        assert_eq!(recovered, input);
    }

    #[test]
    fn round_trips_small_input_uses_no_fsst_form() {
        let input = b"hello world hello world";
        let c = FsstBrotliCodec;
        let compressed = c.compress(input).expect("compress");
        // First 4 bytes are fsst_len; should be 0 for small input.
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&compressed[..4]);
        assert_eq!(u32::from_le_bytes(len_bytes), 0);
        let recovered = c
            .decompress(&compressed, input.len() as u32)
            .expect("decompress");
        assert_eq!(recovered.as_slice(), input);
    }
}
