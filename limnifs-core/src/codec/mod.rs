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
//! | 0x03 | xz    | yes (`omnizip-lzma`) | yes (`omnizip-lzma`) | LZMA2 in XZ container |
//! | 0x04 | brotli | yes (`brotli` q11) | yes (`brotli`) | Best ratio; pure Rust |
//! | 0x05 | deflate | yes (`miniz_oxide`) | yes (`miniz_oxide`) | RFC 1951; universal interop; pure Rust |
//! | 0x06 | snappy | yes (`omnizip-snappy`) | yes (`omnizip-snappy`) | Google's high-speed codec; pure Rust |
//!
//! **100% pure Rust.** No C libraries. Air-gapped safe.

mod bcj_composites;
mod bitshuffle_lz4;
mod brotli;
mod bzip2;
mod composite;
mod deflate;
mod deflate64;
mod flac;
mod fsst_brotli;
mod glza;
mod lz4;
mod ppmd;
mod ppmd8;
mod ricepp;
mod shuffle_lz4;
mod shuffle_zstd;
mod snappy;
mod store;
mod xz;
mod zpaq;
mod zstd;
pub mod zstd_dict;

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
/// Codec id 0x03: XZ/LZMA2 format via `omnizip-lzma`.
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
/// Codec id 0x07: FLAC for PCM audio. **RESERVED** — pending
/// `omnizip-flac` encoder port. The wrapper at `codec::flac::FlacCodec`
/// returns `UnsupportedFeature` until the real codec lands.
pub const CODEC_FLAC: u8 = 0x07;
/// Codec id 0x08: Rice++ for FITS / scientific integer-pixel images.
/// **RESERVED** — pending `omnizip-ricepp` encoder port.
pub const CODEC_RICEPP: u8 = 0x08;
/// Codec id 0x09: FSST + Brotli composite for CSV/JSON.
pub const CODEC_FSST_BROTLI: u8 = 0x09;
/// Codec id 0x0A: BLOSC shuffle + LZ4 for scientific float data.
pub const CODEC_BLOSC2_SHUFFLE_LZ4: u8 = 0x0A;
/// Codec id 0x0B: ZPAQ context-mixing archiver.
pub const CODEC_ZPAQ: u8 = 0x0B;
/// Codec id 0x0C: `PPMd` (dormant — raw fallback).
pub const CODEC_PPMD: u8 = 0x0C;
/// Codec id 0x0D: GLZA grammar-based LZ.
pub const CODEC_GLZA: u8 = 0x0D;
/// Codec id 0x0E: Shuffle+Zstd (BLOSC2 byte-shuffle + Zstd back-end).
pub const CODEC_SHUFFLE_ZSTD: u8 = 0x0E;
/// Codec id 0x0F: Bitshuffle+LZ4 (BLOSC2 bit-shuffle + LZ4 back-end).
pub const CODEC_BITSHUFFLE_LZ4: u8 = 0x0F;
/// Codec id 0x10: `BZip2`.
pub const CODEC_BZIP2: u8 = 0x10;
/// Codec id 0x11: Deflate64 (ZIP method 9, 64 KB window).
pub const CODEC_DEFLATE64: u8 = 0x11;
/// Codec id 0x12: PPMd8 (RESTART + RLE, user-tunable memory budget).
pub const CODEC_PPMD8: u8 = 0x12;

/// Codec id 0x13: LZ4 HC (hash-chain match finder + lazy parsing).
/// Real encoder from omnizip-lz4 0.14.1; was a stub in 0.13.1.
pub const CODEC_LZ4_HC: u8 = 0x13;

/// Codec id 0x20: BCJ-x86 filter + LZ4. For x86/x86_64 executables.
pub const CODEC_BCJ_X86_LZ4: u8 = 0x20;
/// Codec id 0x21: BCJ-x86 filter + ZSTD.
pub const CODEC_BCJ_X86_ZSTD: u8 = 0x21;
/// Codec id 0x23: BCJ-ARM64 filter + LZ4. For AArch64 executables.
pub const CODEC_BCJ_ARM64_LZ4: u8 = 0x23;
/// Codec id 0x24: BCJ-ARM64 filter + ZSTD.
pub const CODEC_BCJ_ARM64_ZSTD: u8 = 0x24;

/// Codec-agnostic tunables. Every codec reads only the fields it
/// understands; the rest are ignored. The struct is the
/// single source of truth for "what knobs does the writer want to
/// turn" — adding a new knob is one field here, not a new
/// `compress_with_*` function per codec (OCP).
#[derive(Clone, Debug)]
pub struct CodecTunables {
    /// Brotli quality (0..=11) and ZSTD level proxy (1..=22).
    /// Codecs without a quality parameter ignore this.
    pub quality: u8,
    /// PPMd7 / PPMd8 context-model order (1..=16).
    pub ppmd_order: u8,
    /// PPMd7 context-tree memory budget in bytes. 0 = codec default.
    pub ppmd7_budget: usize,
    /// PPMd8 context-tree memory budget in bytes. 0 = codec default.
    pub ppmd8_budget: usize,
    /// BZip2 block size in KB (100..=900). Maps to level 1..=9.
    pub bzip2_block_kb: u32,
    /// LZMA dictionary size in MB. Reserved — no pure-Rust LZMA
    /// encoder exists yet; field is here so profiles can declare
    /// intent and we wire it when omnizip-lzma ships an encoder.
    pub lzma_dict_mb: u32,
}

impl CodecTunables {
    /// Build tunables carrying only `quality`. Codecs that don't
    /// override `compress_with_tunables` see no difference from
    /// `compress(plaintext)`.
    #[must_use]
    pub fn from_quality(quality: u8) -> Self {
        Self {
            quality,
            ppmd_order: 0,
            ppmd7_budget: 0,
            ppmd8_budget: 0,
            bzip2_block_kb: 0,
            lzma_dict_mb: 0,
        }
    }
}

impl Default for CodecTunables {
    fn default() -> Self {
        Self {
            quality: 0,
            ppmd_order: 0,
            ppmd7_budget: 0,
            ppmd8_budget: 0,
            bzip2_block_kb: 0,
            lzma_dict_mb: 0,
        }
    }
}

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

    /// Minimum input size for this codec to be tried in the compression
    /// tournament. Chunks smaller than this skip the codec entirely.
    /// Defaults to 0 (no threshold). Override in codec impls that have
    /// significant per-call setup cost (context model initialization,
    /// grammar construction, etc.).
    fn min_compress_size(&self) -> usize {
        0
    }

    /// Compress with a tunables hint. Codecs that have user-tunable
    /// parameters (PPMd order/budget, Brotli quality, ZSTD level,
    /// Bzip2 block size, …) override this; the default impl ignores
    /// tunables and calls `compress`. Adding a tunable is therefore
    /// backward-compatible — old callers keep working.
    ///
    /// # Errors
    ///
    /// Same as [`Codec::compress`].
    fn compress_with_tunables(
        &self,
        plaintext: &[u8],
        tunables: &CodecTunables,
    ) -> Result<Vec<u8>, CoreError> {
        let _ = tunables;
        self.compress(plaintext)
    }
}

/// Optional trait: codecs with strongly-typed per-codec tunables.
///
/// The flat [`CodecTunables`] struct works for today's six codec
/// families but doesn't scale. Codecs that want clean OCP for their
/// own knobs implement this trait alongside [`Codec`]; new codecs
/// = one `impl PerCodecTunables` with a fresh `Tunables` type, no
/// edits to existing code or to the flat struct.
///
/// The flat `CodecTunables` remains the dispatch entry point for
/// callers that want a single uniform struct; codecs that implement
/// `PerCodecTunables` can read from it inside their
/// `compress_with_tunables` override.
pub trait PerCodecTunables: Codec {
    /// Per-codec tunables type. Should be `Clone + Send + Sync +
    /// 'static` so it can live in a `Box<dyn Any>` registry if/when
    /// we move to per-codec-keyed tunables dispatch.
    type Tunables: Clone + Send + Sync + 'static;

    /// Compress with this codec's strongly-typed tunables.
    ///
    /// # Errors
    ///
    /// Same as [`Codec::compress`].
    fn compress_with_owned_tunables(
        &self,
        plaintext: &[u8],
        tunables: &Self::Tunables,
    ) -> Result<Vec<u8>, CoreError>;
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
    /// registered.
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

    /// Dispatch compression with a tunables hint. Codecs that don't
    /// override the trait method fall through to plain `compress`.
    ///
    /// # Errors
    ///
    /// Same as [`CodecRegistry::compress`].
    pub fn compress_with_tunables(
        &self,
        id: u8,
        plaintext: &[u8],
        tunables: &CodecTunables,
    ) -> Result<Vec<u8>, CoreError> {
        match self.find(id) {
            Some(codec) => codec.compress_with_tunables(plaintext, tunables),
            None => Err(CoreError::UnsupportedFeature {
                feature: format!(
                    "compress_with_tunables codec 0x{id:02X} (registered: {registered})",
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
        registry.register(Box::new(lz4::Lz4HcCodec));
        registry.register(Box::new(zstd::ZstdCodec));
        registry.register(Box::new(xz::XzCodec));
        registry.register(Box::new(brotli::BrotliCodec));
        registry.register(Box::new(deflate::DeflateCodec));
        registry.register(Box::new(snappy::SnappyCodec));
        // Reserved stubs — wire-format ids exist; codecs pending omnizip ports.
        // Registered so `compress(CODEC_FLAC, ...)` surfaces a clear
        // "codec 0x07 awaiting omnizip-flac" instead of "0x07 not
        // registered". Categorizers can detect this and fall back
        // gracefully.
        registry.register(Box::new(flac::FlacCodec));
        registry.register(Box::new(ricepp::RiceppCodec::fits_default()));
        registry.register(Box::new(fsst_brotli::FsstBrotliCodec));
        registry.register(Box::new(shuffle_lz4::ShuffleLz4Codec::float32()));
        registry.register(Box::new(zpaq::ZpaqCodec));
        registry.register(Box::new(ppmd::PpmdCodec::new()));
        registry.register(Box::new(ppmd8::Ppmd8Codec::new()));
        registry.register(Box::new(glza::GlzaCodec));
        registry.register(Box::new(shuffle_zstd::ShuffleZstdCodec::new()));
        registry.register(Box::new(bitshuffle_lz4::BitshuffleLz4Codec::new()));
        registry.register(Box::new(bzip2::Bzip2Codec::new()));
        registry.register(Box::new(deflate64::Deflate64Codec::new()));
        // BCJ composite codecs — filter executable code then compress.
        // Categorizer picks the right one based on ELF/PE/Mach-O
        // architecture (see TODO.impl/04-bcj-categorizer-routing.md).
        registry.register(Box::new(bcj_composites::BcjX86Lz4Codec));
        registry.register(Box::new(bcj_composites::BcjX86ZstdCodec));
        registry.register(Box::new(bcj_composites::BcjArm64Lz4Codec));
        registry.register(Box::new(bcj_composites::BcjArm64ZstdCodec));
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

/// Returns the best available codec for compressible content classes
/// (Text, Code). Brotli q5 is the current default — beats ZSTD L6
/// (omnizip 0.7+) on real source code in our benchmarks. Try
/// switching to `CODEC_ZSTD` if ZSTD's level differentiation
/// improves enough to beat Brotli; the change is one line.
#[must_use]
pub fn best_compressible_codec() -> u8 {
    CODEC_BROTLI
}

/// Returns the best available codec for binary content classes
/// (structured binary — ELF, Mach-O, PE, object files, etc.).
///
/// LZ4 is the right choice in the current registry: ruzstd's encoder
/// is level-1-only and produces output roughly the size of the input,
/// so ZSTD effectively means "store with extra overhead". LZ4 gives
/// 1.5–2× on structured binary at multiple-GB/s encode speed.
///
/// Will switch back to ZSTD once `omnizip-zstd` ships a real encoder
/// (Phase C, tracked in `omnizip/omnizip-rs`).
#[must_use]
pub fn best_binary_codec() -> u8 {
    CODEC_LZ4
}

/// Compress `plaintext` using the codec identified by `codec_id`, via
/// the process-wide default [`CodecRegistry`].
///
/// # Errors
///
/// Returns [`CoreError::UnsupportedFeature`] for unknown codec ids.
/// Returns [`CoreError::Corrupt`] if the encoder fails.
pub fn compress(codec_id: u8, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    default_registry().compress(codec_id, plaintext)
}

/// Compress with a quality/level hint. For codecs that support a
/// quality parameter (Brotli, ZSTD), this overrides the default.
/// For codecs without quality control (LZ4, Store, Snappy), the
/// hint is silently ignored.
///
/// `quality` interpretation per codec:
/// - Brotli (0x04): 0..=11 (higher = better ratio, slower)
/// - ZSTD (0x02): 1..=22 (higher = better ratio, slower)
/// - All others: ignored
///
/// For PPMd7 / PPMd8 / Bzip2 tunables, use
/// [`compress_with_tunables`] with a fully-populated
/// [`CodecTunables`].
///
/// # Errors
/// Same as [`compress`].
pub fn compress_with_options(
    codec_id: u8,
    plaintext: &[u8],
    quality: u8,
) -> Result<Vec<u8>, CoreError> {
    let tunables = CodecTunables::from_quality(quality);
    compress_with_tunables(codec_id, plaintext, &tunables)
}

/// Compress `plaintext` with the given codec and tunables. Codecs
/// that don't override `compress_with_tunables` on the [`Codec`]
/// trait fall back to plain `compress`.
///
/// # Errors
///
/// Same as [`compress`].
pub fn compress_with_tunables(
    codec_id: u8,
    plaintext: &[u8],
    tunables: &CodecTunables,
) -> Result<Vec<u8>, CoreError> {
    default_registry().compress_with_tunables(codec_id, plaintext, tunables)
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

/// Compress with Brotli at an explicit quality (0–11). Quality 0 is
/// the fastest; quality 11 is the reference encoder's maximum.
///
/// This bypasses the codec registry's per-codec default and is the
/// right call for callers that know they want Brotli at a specific
/// quality — e.g. the writer's metadata-blob path, which often
/// compresses multi-MiB blobs where the default q5 is the
/// bottleneck.
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if the Brotli encoder fails.
pub fn compress_brotli_with_quality(plaintext: &[u8], quality: i32) -> Result<Vec<u8>, CoreError> {
    brotli::compress(plaintext, quality)
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
    fn tunables_ppmd7_bigger_budget_helps_ratio() {
        // Synthetic but realistic: a 1 MB text fixture with mixed
        // repetition. PPMd7 with 256 MB context budget should
        // outperform the 8 MB default.
        let mut input = Vec::with_capacity(1 * 1024 * 1024);
        let paragraph = b"the quick brown fox jumps over the lazy dog. ";
        while input.len() + paragraph.len() <= 1 * 1024 * 1024 {
            input.extend_from_slice(paragraph);
        }

        let small = CodecTunables {
            quality: 0,
            ppmd_order: 4,
            ppmd7_budget: 8 * 1024 * 1024,
            ppmd8_budget: 0,
            bzip2_block_kb: 0,
            lzma_dict_mb: 0,
        };
        let big = CodecTunables {
            ppmd7_budget: 256 * 1024 * 1024,
            ..small.clone()
        };

        let small_c = compress_with_tunables(CODEC_PPMD, &input, &small).expect("ppmd7 small");
        let big_c = compress_with_tunables(CODEC_PPMD, &input, &big).expect("ppmd7 big");
        assert!(
            big_c.len() <= small_c.len(),
            "256MB budget should not be worse than 8MB ({} vs {})",
            big_c.len(),
            small_c.len()
        );

        // Round trip.
        let recovered = decompress(CODEC_PPMD, &small_c, input.len() as u32).expect("d");
        assert_eq!(recovered, input);
    }

    #[test]
    fn tunables_brotli_quality_flows_through() {
        // Mixed natural-language text — q11's larger window and
        // context model should beat q0's "store literals" mode.
        let paragraph = b"Lorem ipsum dolor sit amet, consectetur adipiscing elit, \
                          sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. \
                          Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris.";
        let mut input = Vec::with_capacity(200_000);
        let mut i = 0;
        while input.len() < 200_000 {
            // Vary each line slightly so q0 can't just RLE the whole thing.
            input.extend_from_slice(format!("{i:04}: {paragraph:?}\n").as_bytes());
            i += 1;
        }
        let q0 = CodecTunables::from_quality(0);
        let q11 = CodecTunables::from_quality(11);
        let c0 = compress_with_tunables(CODEC_BROTLI, &input, &q0).expect("brotli q0");
        let c11 = compress_with_tunables(CODEC_BROTLI, &input, &q11).expect("brotli q11");
        assert!(
            c11.len() < c0.len(),
            "q11 ({}) should beat q0 ({}) on mixed text",
            c11.len(),
            c0.len()
        );
    }

    #[test]
    fn tunables_bzip2_block_size_maps_to_level() {
        let input = b"the quick brown fox jumps over the lazy dog. ".repeat(2000);
        let small = CodecTunables {
            bzip2_block_kb: 100,
            ..CodecTunables::default()
        };
        let big = CodecTunables {
            bzip2_block_kb: 900,
            ..CodecTunables::default()
        };
        let cs = compress_with_tunables(CODEC_BZIP2, &input, &small).expect("bzip2 100k");
        let cb = compress_with_tunables(CODEC_BZIP2, &input, &big).expect("bzip2 900k");
        assert!(
            cb.len() <= cs.len(),
            "900k ({}) <= 100k ({})",
            cb.len(),
            cs.len()
        );
    }

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
    fn zstd_higher_levels_compress_better_than_lower() {
        // Regression for the omnizip 0.5→0.7 ZSTD level differentiation
        // fix. omnizip 0.5 produced identical output for all 5 levels;
        // 0.7 must differentiate.
        //
        // 0.14.8 had a regression where Default (L6) and higher produced
        // pathological output on this input (50 KB+ and 14+ s). 0.14.10
        // (omnizip-rs PR #90) fixes it; this test stays as a guard
        // against future regressions. See
        // `docs/omnizip-proposals/zstd-default-broken.md`.
        let input: Vec<u8> = b"The quick brown fox jumps over the lazy dog. ".repeat(2000);
        let l1 = omnizip_zstd::compress(&input, omnizip_zstd::ZstdLevel::Fastest).expect("zstd L1");
        let l6 = omnizip_zstd::compress(&input, omnizip_zstd::ZstdLevel::Default).expect("zstd L6");
        assert!(
            l6.len() < l1.len(),
            "ZSTD L6 ({}) should beat L1 ({}); level differentiation broken",
            l6.len(),
            l1.len()
        );
    }

    #[test]
    fn xz_lzma_round_trips_via_lazy_parsing() {
        // Regression for the omnizip 0.5→0.7 LZMA lazy-parsing rewrite.
        // We don't assert LZMA beats ZSTD on synthetic-repetitive input
        // (extreme inputs hit edge cases in the encoder), only that
        // real-world text round-trips through the new encoder.
        let input: Vec<u8> = b"The quick brown fox jumps over the lazy dog. \
                               Lorem ipsum dolor sit amet. \
                               SVG is a vector image format."
            .repeat(500);
        let xz = omnizip_lzma::xz_compress(&input).expect("xz encode");
        let recovered = omnizip_lzma::xz_container::xz_decompress(&xz).expect("xz decode");
        assert_eq!(recovered, input);
        assert!(
            xz.len() < input.len(),
            "LZMA should compress real-world text; got {} vs {}",
            xz.len(),
            input.len()
        );
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
    fn xz_encode_round_trips() {
        // omnizip-lzma's xz_compress is Phase B (literal-only) so the
        // output is larger than the input, but it must round-trip
        // through the LZMA2 decoder.
        let plaintext = b"xz round-trip data";
        let compressed = compress(CODEC_XZ, plaintext).expect("xz encode succeeds");
        let decompressed =
            decompress(CODEC_XZ, &compressed, plaintext.len() as u32).expect("xz decode succeeds");
        assert_eq!(decompressed.as_slice(), plaintext);
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
