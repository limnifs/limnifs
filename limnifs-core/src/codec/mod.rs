//! Codec registry — dispatches compression/decompression by codec id.
//!
//! Each drop record carries a `representation` triple `(codec, aead, ec)`.
//! This module centralises codec dispatch behind a [`Codec`] trait and a
//! [`CodecRegistry`], so adding a codec is a new file + one registration
//! call (open/closed). The existing free functions [`compress`] and
//! [`decompress`] remain as thin wrappers around the default registry.
//!
//! ## Supported codecs
//!
//! | Id  | Name   | Encode | Decode | Notes |
//! |-----|--------|--------|--------|-------|
//! | 0x00 | store | yes (identity) | yes | No compression |
//! | 0x01 | lz4   | yes (`lz4_flex`) | yes | Fast baseline; pure Rust |
//! | 0x02 | zstd  | yes (`ruzstd` `Fastest`) | yes (`ruzstd`) | Pure Rust; ZSTD level 1 |
//! | 0x03 | xz    | **no** | yes (`lzma-rs`) | Decode-only for legacy drops |
//! | 0x04 | brotli | yes (`brotli` q11) | yes (`brotli`) | Best ratio; pure Rust |
//! | 0x05 | deflate | yes (`miniz_oxide`) | yes (`miniz_oxide`) | RFC 1951; universal interop; pure Rust |
//! | 0x06 | snappy | yes (`omnizip-snappy`) | yes (`omnizip-snappy`) | Google's high-speed codec; pure Rust |
//!
//! **Why XZ is decode-only.** `lzma-rs` 0.3.0 ships an LZMA2 "encoder" that
//! wraps input as uncompressed chunks (`encode/lzma2.rs`) and a raw-LZMA
//! encoder that emits literals only (`encode/dumbencoder.rs`). Neither
//! performs real compression. There is no mature pure-Rust LZMA encoder as
//! of 2026, so `LimniFS` reserves the XZ codec id for reading legacy drops
//! produced by external tooling and routes its own encoding to ZSTD.
//!
//! **100% pure Rust.** No C libraries. Air-gapped safe.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

mod brotli;
mod deflate;
mod lz4;
mod snappy;
mod store;
mod xz;
mod zstd;

use std::sync::OnceLock;

use crate::error::CoreError;

/// Codec id 0x00: store (no compression).
pub const CODEC_STORE: u8 = 0x00;
/// Codec id 0x01: LZ4 block format (`lz4_flex`, pure Rust).
pub const CODEC_LZ4: u8 = 0x01;
/// Codec id 0x02: Zstandard frame format (`ruzstd`, pure Rust).
/// Encode uses `CompressionLevel::Fastest` (ZSTD level 1); decode supports
/// any level the reference encoder can produce.
pub const CODEC_ZSTD: u8 = 0x02;
/// Codec id 0x03: XZ/LZMA2 format. Decode-only in pure Rust (`lzma-rs`).
pub const CODEC_XZ: u8 = 0x03;
/// Codec id 0x04: Brotli frame format (`brotli`, pure Rust). Encode at
/// quality 11 (best ratio); decode at any quality.
pub const CODEC_BROTLI: u8 = 0x04;
/// Codec id 0x05: DEFLATE stream format (`miniz_oxide`, pure Rust).
/// Raw RFC 1951 inside a zlib wrapper (RFC 1950).
pub const CODEC_DEFLATE: u8 = 0x05;
/// Codec id 0x06: Snappy format (`omnizip-snappy` → `snap`, pure Rust).
/// No compression levels; ~500 MB/s encode and decode.
pub const CODEC_SNAPPY: u8 = 0x06;

/// The behaviour every compression codec implements. New codecs register
/// a `Codec` impl with [`CodecRegistry::register`]; the dispatch code
/// never changes.
pub trait Codec: Send + Sync {
    /// The wire-format codec id recorded in the drop record.
    fn id(&self) -> u8;
    /// Human-readable name for diagnostics.
    fn name(&self) -> &'static str;
    /// Compress `plaintext` into the codec's wire format.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::UnsupportedFeature`] if the codec is
    /// decode-only in pure Rust (currently only XZ), or
    /// [`CoreError::Corrupt`] if the encoder fails.
    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError>;
    /// Decompress `compressed`, verifying the output length matches
    /// `expected_len` exactly.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::Corrupt`] if decompression fails or the
    /// result length does not match `expected_len`.
    fn decompress(&self, compressed: &[u8], expected_len: u32) -> Result<Vec<u8>, CoreError>;
}

/// Process-wide registry of codecs, keyed by codec id.
pub struct CodecRegistry {
    codecs: Vec<Box<dyn Codec>>,
}

impl CodecRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self { codecs: Vec::new() }
    }

    /// Register a codec. Id collisions are rejected at runtime — two codecs
    /// claiming the same id is a programming error, not a recoverable
    /// condition.
    ///
    /// # Panics
    ///
    /// Panics if a codec with the same id is already registered.
    pub fn register(&mut self, codec: Box<dyn Codec>) {
        let id = codec.id();
        assert!(
            !self.codecs.iter().any(|c| c.id() == id),
            "codec id 0x{id:02X} already registered",
        );
        self.codecs.push(codec);
    }

    fn find(&self, id: u8) -> Option<&dyn Codec> {
        self.codecs.iter().find(|c| c.id() == id).map(Box::as_ref)
    }

    fn registered_names(&self) -> String {
        self.codecs
            .iter()
            .map(|c| format!("0x{:02X}={}", c.id(), c.name()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Dispatch compression to the codec identified by `id`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::UnsupportedFeature`] if no codec with `id` is
    /// registered, or if the codec is decode-only.
    pub fn compress(&self, id: u8, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        match self.find(id) {
            Some(codec) => codec.compress(plaintext),
            None => Err(CoreError::UnsupportedFeature {
                feature: format!(
                    "compress codec 0x{id:02X} (registered: {registered})",
                    registered = self.registered_names()
                ),
            }),
        }
    }

    /// Dispatch decompression to the codec identified by `id`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::UnsupportedFeature`] if no codec with `id` is
    /// registered, or [`CoreError::Corrupt`] if decompression fails.
    pub fn decompress(
        &self,
        id: u8,
        compressed: &[u8],
        expected_len: u32,
    ) -> Result<Vec<u8>, CoreError> {
        match self.find(id) {
            Some(codec) => codec.decompress(compressed, expected_len),
            None => Err(CoreError::UnsupportedFeature {
                feature: format!(
                    "decompress codec 0x{id:02X} (registered: {registered})",
                    registered = self.registered_names()
                ),
            }),
        }
    }
}

impl Default for CodecRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(store::StoreCodec));
        registry.register(Box::new(lz4::Lz4Codec));
        registry.register(Box::new(zstd::ZstdCodec));
        registry.register(Box::new(xz::XzCodec));
        registry.register(Box::new(brotli::BrotliCodec));
        registry.register(Box::new(deflate::DeflateCodec));
        registry.register(Box::new(snappy::SnappyCodec));
        registry
    }
}

impl std::fmt::Debug for CodecRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodecRegistry")
            .field("codecs", &self.registered_names())
            .finish()
    }
}

static DEFAULT_REGISTRY: OnceLock<CodecRegistry> = OnceLock::new();

fn default_registry() -> &'static CodecRegistry {
    DEFAULT_REGISTRY.get_or_init(CodecRegistry::default)
}

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

/// Compress `plaintext` using the codec identified by `codec_id`, via
/// the process-wide default [`CodecRegistry`].
///
/// # Errors
///
/// Returns [`CoreError::UnsupportedFeature`] for unknown codec ids and
/// for codecs that are decode-only in pure Rust (currently `CODEC_XZ`).
/// Returns [`CoreError::Corrupt`] if the encoder fails.
pub fn compress(codec_id: u8, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    default_registry().compress(codec_id, plaintext)
}

/// Decompress `compressed` using the codec identified by `codec_id`, via
/// the process-wide default [`CodecRegistry`]. The `expected_len` is the
/// `plaintext_len` from the drop record; the decompressed output MUST
/// match it exactly.
///
/// # Errors
///
/// Returns [`CoreError::UnsupportedFeature`] for unknown codec ids.
/// Returns [`CoreError::Corrupt`] if decompression fails or the result
/// length does not match `expected_len`.
pub fn decompress(
    codec_id: u8,
    compressed: &[u8],
    expected_len: u32,
) -> Result<Vec<u8>, CoreError> {
    default_registry().decompress(codec_id, compressed, expected_len)
}

/// Compress with LZ4, prepending the original size as a 4-byte LE
/// header (the format `lz4_flex::decompress_size_prepended` expects).
#[must_use]
pub fn compress_lz4_with_size(plaintext: &[u8]) -> Vec<u8> {
    lz4::compress_lz4_with_size(plaintext)
}

/// Compress with Zstandard at `CompressionLevel::Fastest` (ZSTD level 1).
/// The output is a standard ZSTD frame decodable by any conformant ZSTD
/// decoder.
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if the ZSTD encoder fails.
pub fn compress_zstd(plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    zstd::compress(plaintext)
}

/// Compress with Brotli at quality 11 (best ratio).
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if the Brotli encoder fails.
pub fn compress_brotli(plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    brotli::compress(plaintext, brotli::DEFAULT_QUALITY)
}

/// Compress with DEFLATE at level 6 (default). Output is a zlib-framed
/// DEFLATE stream (RFC 1950) decodable by any zlib decoder (`gzip -d`,
/// `zlib.decompress`, etc.).
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if the DEFLATE encoder fails (rare).
pub fn compress_deflate(plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    deflate::compress(plaintext, deflate::DEFAULT_LEVEL)
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
        let data: Vec<u8> = (0..100_000u32)
            .map(|i| u8::try_from(i % 256).expect("fits u8"))
            .collect();
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

    #[test]
    fn brotli_round_trips() {
        let data = b"The quick brown fox jumps over the lazy dog. ".repeat(1000);
        let compressed = compress_brotli(&data).expect("brotli compress");
        let decompressed = decompress(
            CODEC_BROTLI,
            &compressed,
            u32::try_from(data.len()).expect("fits u32"),
        )
        .expect("brotli decompress");
        assert_eq!(decompressed, data);
    }

    #[test]
    fn brotli_compresses_repetitive_data() {
        let data = vec![0x41u8; 10_000];
        let compressed = compress_brotli(&data).expect("brotli compress");
        assert!(
            compressed.len() < data.len(),
            "brotli should compress repetitive data: {} vs {}",
            compressed.len(),
            data.len()
        );
    }

    #[test]
    fn brotli_beats_zstd_on_text() {
        let data = b"The quick brown fox. ".repeat(10_000);
        let zstd = compress_zstd(&data).expect("zstd");
        let br = compress_brotli(&data).expect("brotli");
        assert!(
            br.len() < zstd.len(),
            "brotli q11 ({}) should beat zstd-1 ({}) on text",
            br.len(),
            zstd.len()
        );
    }

    #[test]
    fn brotli_decompress_rejects_length_mismatch() {
        let data = b"hello world";
        let compressed = compress_brotli(data).expect("brotli compress");
        match decompress(CODEC_BROTLI, &compressed, 99) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("does not match"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn deflate_round_trips() {
        let data = b"The quick brown fox jumps over the lazy dog. ".repeat(1000);
        let compressed = compress_deflate(&data).expect("deflate compress");
        let decompressed = decompress(
            CODEC_DEFLATE,
            &compressed,
            u32::try_from(data.len()).expect("fits u32"),
        )
        .expect("deflate decompress");
        assert_eq!(decompressed, data);
    }

    #[test]
    fn deflate_compresses_repetitive_data() {
        let data = vec![0x41u8; 10_000];
        let compressed = compress_deflate(&data).expect("deflate compress");
        assert!(
            compressed.len() < data.len(),
            "deflate should compress repetitive data: {} vs {}",
            compressed.len(),
            data.len()
        );
    }

    #[test]
    fn deflate_decompress_rejects_length_mismatch() {
        let data = b"hello world";
        let compressed = compress_deflate(data).expect("deflate compress");
        match decompress(CODEC_DEFLATE, &compressed, 99) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("does not match"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn snappy_round_trips() {
        let data = b"The quick brown fox jumps over the lazy dog. ".repeat(100);
        let compressed = compress(CODEC_SNAPPY, &data).expect("snappy compress");
        let decompressed = decompress(
            CODEC_SNAPPY,
            &compressed,
            u32::try_from(data.len()).expect("fits u32"),
        )
        .expect("snappy decompress");
        assert_eq!(decompressed, data);
    }

    #[test]
    fn snappy_compresses_repetitive_data() {
        let data = vec![0x41u8; 10_000];
        let compressed = compress(CODEC_SNAPPY, &data).expect("snappy compress");
        assert!(
            compressed.len() < data.len(),
            "snappy should compress repetitive data: {} vs {}",
            compressed.len(),
            data.len()
        );
    }

    #[test]
    fn snappy_decompress_rejects_length_mismatch() {
        let data = b"hello world";
        let compressed = compress(CODEC_SNAPPY, data).expect("snappy compress");
        match decompress(CODEC_SNAPPY, &compressed, 99) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(
                    reason.contains("length mismatch") || reason.contains("does not match"),
                    "got: {reason}"
                );
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn registry_registers_custom_codec_without_changing_dispatch() {
        struct NoopCodec;
        const NOOP_ID: u8 = 0xFE;
        impl Codec for NoopCodec {
            fn id(&self) -> u8 {
                NOOP_ID
            }
            fn name(&self) -> &'static str {
                "noop"
            }
            fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
                Ok(plaintext.to_vec())
            }
            fn decompress(
                &self,
                compressed: &[u8],
                expected_len: u32,
            ) -> Result<Vec<u8>, CoreError> {
                let expected = usize::try_from(expected_len).map_err(|_| CoreError::Corrupt {
                    reason: format!("noop: expected_len {expected_len} exceeds usize"),
                })?;
                if compressed.len() != expected {
                    return Err(CoreError::Corrupt {
                        reason: "noop: length mismatch".into(),
                    });
                }
                Ok(compressed.to_vec())
            }
        }

        let mut registry = CodecRegistry::new();
        registry.register(Box::new(NoopCodec));
        assert_eq!(registry.compress(NOOP_ID, b"abc").expect("noop"), b"abc");
        assert_eq!(
            registry
                .decompress(NOOP_ID, b"abc", 3)
                .expect("noop decompress"),
            b"abc"
        );
    }

    #[test]
    #[should_panic(expected = "codec id 0x00 already registered")]
    fn registry_rejects_duplicate_id() {
        let mut registry = CodecRegistry::new();
        registry.register(Box::new(store::StoreCodec));
        registry.register(Box::new(store::StoreCodec));
    }

    #[test]
    fn default_registry_has_all_seven_codecs() {
        let registry = default_registry();
        assert!(registry.find(CODEC_STORE).is_some());
        assert!(registry.find(CODEC_LZ4).is_some());
        assert!(registry.find(CODEC_ZSTD).is_some());
        assert!(registry.find(CODEC_XZ).is_some());
        assert!(registry.find(CODEC_BROTLI).is_some());
        assert!(registry.find(CODEC_DEFLATE).is_some());
        assert!(registry.find(CODEC_SNAPPY).is_some());
        assert!(registry.find(0xFF).is_none());
    }
}
