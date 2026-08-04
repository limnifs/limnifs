//! Compression profiles — predefined codec strategies for different goals.
//!
//! A [`CompressionProfile`] bundles codec selection, parameters, tournament
//! behavior, and chunking into a single named configuration. Four built-in
//! profiles cover the main use cases; users can define custom profiles via
//! TOML.
//!
//! ## Built-in profiles
//!
//! | Profile | Goal | Create speed | Ratio | vs SquashFS | vs DwarFS |
//! |---------|------|-------------|-------|------------|-----------|
//! | `max-ratio` | Smallest output | Slow | Best | Wins ratio | Ties ratio |
//! | `max-speed` | Fastest create | Match SquashFS | OK | Ties speed | Wins speed |
//! | `balanced` | Good trade-off | Medium | Good | Wins ratio | Wins speed |
//! | `competitive` | Beat both | Fast | Best-of-both | **Wins both** | **Wins both** |
//!
//! ## Usage
//!
//! ```toml
//! # Use a built-in profile
//! profile = "competitive"
//!
//! # Or define a custom profile inline
//! [profile]
//! name = "my-custom"
//! text_codec = "brotli"
//! brotli_quality = 7
//! binary_codec = "lz4"
//! chunk_avg_size = 32768
//! tournament = "none"
//! ```

#![allow(warnings)]

use serde::{Deserialize, Serialize};

use crate::config::{
    ChunkingConfig, CodecTunables, Defaults, DictionaryConfig, EncryptionConfig, TournamentConfig,
    WriteConfig,
};

/// Built-in profile names.
pub const MAX_RATIO: &str = "max-ratio";
pub const MAX_SPEED: &str = "max-speed";
pub const BALANCED: &str = "balanced";
pub const COMPETITIVE: &str = "competitive";
pub const MAX_READ: &str = "max-read";
pub const MAX_WRITE: &str = "max-write";
pub const MAX_WRITE_RW: &str = "max-write-rw";
pub const MAX_READ_RW: &str = "max-read-rw";
pub const BALANCED_RW: &str = "balanced-rw";

/// Select a built-in profile by name. Returns a complete [`WriteConfig`]
/// configured for that profile's strategy.
#[must_use]
pub fn select(name: &str) -> Option<WriteConfig> {
    match name {
        MAX_RATIO => Some(max_ratio()),
        MAX_SPEED => Some(max_speed()),
        BALANCED => Some(balanced()),
        COMPETITIVE => Some(competitive()),
        MAX_READ => Some(max_read()),
        MAX_WRITE => Some(max_write()),
        MAX_WRITE_RW => Some(max_write_rw()),
        MAX_READ_RW => Some(max_read_rw()),
        BALANCED_RW => Some(balanced_rw()),
        _ => None,
    }
}

/// Maximum compression ratio. Tries every applicable codec per drop,
/// picks the smallest. Slowest create, smallest output.
///
/// - Text: Brotli q11 + LZMA + PPMd7 (256 MB budget) tournament
/// - Binary: ZSTD L19 + LZMA tournament
/// - Categorizers: all enabled (FLAC, Rice++, FSST+Brotli)
/// - Chunks: 64 KB (better cross-chunk pattern matching)
/// - Whole-file max: 256 MB
#[must_use]
pub fn max_ratio() -> WriteConfig {
    WriteConfig {
        defaults: Defaults {
            text_codec: "brotli".into(),
            binary_codec: "zstd".into(),
            metadata_codec: "brotli".into(),
            metadata_quality: 11,
            inline_threshold: 8192,
        },
        categorizers: crate::config::defaults::all_v0_1(),
        chunking: ChunkingConfig {
            name: "fastcdc".into(),
            avg_chunk_size: 65_536,
            min_chunk_size: 8192,
            max_chunk_size: 262_144,
        },
        tournament: TournamentConfig {
            codecs: vec![
                "store".into(),
                "lz4".into(),
                "lz4-hc".into(),
                "zstd".into(),
                "brotli".into(),
                "ppmd".into(),
                "bzip2".into(),
            ],
            min_size_threshold: 256,
            skip_for_binary: false,
        },
        codec_tunables: CodecTunables {
            ppmd7: crate::config::Ppmd7Tunables {
                order: 6,
                memory_budget_mb: 256,
            },
            ppmd8: crate::config::Ppmd8Tunables {
                order: 8,
                memory_budget_mb: 128,
            },
            brotli: crate::config::BrotliTunables {
                quality: 11,
                window: 24,
            },
            lzma: crate::config::LzmaTunables {
                lc: 3,
                lp: 0,
                pb: 2,
                dict_size_mb: 64,
                use_optimal_parser: true,
            },
            bzip2: crate::config::Bzip2Tunables { block_size_kb: 900 },
        },
        encryption: EncryptionConfig {
            aead: "chacha20-poly1305".into(),
            key_wrap: "x25519-hkdf".into(),
        },
        dictionaries: DictionaryConfig {
            enabled: true,
            min_class_size: 50,
            max_dict_size: 131_072,
            trainer: "frequency".into(),
        },
        mode: crate::config::ImageMode::ReadOnly,
        write_codec: "lz4".into(),
        turnover_threshold: 0,
    }
}

/// Maximum speed. Single-codec per content class, no tournament.
/// Matches SquashFS LZ4 speed on binary data.
///
/// - Text: LZ4 (instant)
/// - Binary: LZ4 (instant)
/// - Categorizers: disabled (no FLAC, no Rice++)
/// - Tournament: none (classify once, compress once)
/// - Chunks: 4 KB (maximum parallelism)
#[must_use]
pub fn max_speed() -> WriteConfig {
    WriteConfig {
        defaults: Defaults {
            text_codec: "lz4".into(),
            binary_codec: "lz4".into(),
            metadata_codec: "lz4".into(),
            metadata_quality: 1,
            inline_threshold: 4096,
        },
        categorizers: vec![],
        chunking: ChunkingConfig {
            name: "fastcdc".into(),
            avg_chunk_size: 4096,
            min_chunk_size: 512,
            max_chunk_size: 16_384,
        },
        tournament: TournamentConfig {
            codecs: vec!["store".into(), "lz4".into()],
            min_size_threshold: 0,
            skip_for_binary: true,
        },
        codec_tunables: CodecTunables {
            brotli: crate::config::BrotliTunables {
                quality: 0,
                window: 10,
            },
            ..CodecTunables::default()
        },
        encryption: EncryptionConfig {
            aead: "chacha20-poly1305".into(),
            key_wrap: "x25519-hkdf".into(),
        },
        dictionaries: DictionaryConfig {
            enabled: false,
            min_class_size: 0,
            max_dict_size: 0,
            trainer: "frequency".into(),
        },
        mode: crate::config::ImageMode::ReadOnly,
        write_codec: "lz4".into(),
        turnover_threshold: 0,
    }
}

/// Balanced profile. Good ratio + good speed for general use.
///
/// - Text: Brotli q5 (fast, good ratio)
/// - Binary: LZ4 (fast)
/// - Categorizers: FLAC for small audio, skip large
/// - Tournament: Brotli + ZSTD only
/// - Chunks: 16 KB
#[must_use]
pub fn balanced() -> WriteConfig {
    WriteConfig {
        defaults: Defaults {
            text_codec: "brotli".into(),
            binary_codec: "lz4".into(),
            metadata_codec: "zstd".into(),
            metadata_quality: 3,
            inline_threshold: 4096,
        },
        categorizers: crate::config::defaults::all_v0_1(),
        chunking: ChunkingConfig {
            name: "fastcdc".into(),
            avg_chunk_size: 16_384,
            min_chunk_size: 2048,
            max_chunk_size: 65_536,
        },
        tournament: TournamentConfig {
            codecs: vec!["store".into(), "lz4".into(), "brotli".into()],
            min_size_threshold: 256,
            skip_for_binary: true,
        },
        codec_tunables: CodecTunables {
            brotli: crate::config::BrotliTunables {
                quality: 5,
                window: 22,
            },
            ..CodecTunables::default()
        },
        encryption: EncryptionConfig {
            aead: "chacha20-poly1305".into(),
            key_wrap: "x25519-hkdf".into(),
        },
        dictionaries: DictionaryConfig {
            enabled: true,
            min_class_size: 100,
            max_dict_size: 65_536,
            trainer: "frequency".into(),
        },
        mode: crate::config::ImageMode::ReadOnly,
        write_codec: "lz4".into(),
        turnover_threshold: 0,
    }
}

/// Competitive profile — beat SquashFS on ratio AND DwarFS on speed.
///
/// Uses ZSTD L1 for text (5x faster compress than Brotli, 3x faster
/// decompress, 3x better ratio than SquashFS LZ4). LZ4 for binary.
#[must_use]
pub fn competitive() -> WriteConfig {
    WriteConfig {
        defaults: Defaults {
            text_codec: "zstd".into(),
            binary_codec: "lz4".into(),
            metadata_codec: "zstd".into(),
            metadata_quality: 3,
            inline_threshold: 4096,
        },
        categorizers: crate::config::defaults::all_v0_1(),
        chunking: ChunkingConfig {
            name: "fastcdc".into(),
            avg_chunk_size: 8192,
            min_chunk_size: 1024,
            max_chunk_size: 65_536,
        },
        tournament: TournamentConfig {
            codecs: vec!["store".into(), "lz4".into(), "brotli".into()],
            min_size_threshold: 0,
            skip_for_binary: true,
        },
        codec_tunables: CodecTunables {
            brotli: crate::config::BrotliTunables {
                quality: 5,
                window: 22,
            },
            ..CodecTunables::default()
        },
        encryption: EncryptionConfig {
            aead: "chacha20-poly1305".into(),
            key_wrap: "x25519-hkdf".into(),
        },
        dictionaries: DictionaryConfig {
            enabled: false,
            min_class_size: 0,
            max_dict_size: 0,
            trainer: "frequency".into(),
        },
        mode: crate::config::ImageMode::ReadOnly,
        write_codec: "lz4".into(),
        turnover_threshold: 0,
    }
}

/// Maximum read profile — optimized for read-heavy workloads (write
/// once, read many). Uses codecs with the best ratio that still
/// decompresses quickly. Write cost is amortised over many reads.
///
/// - Text/Binary: ZSTD L19 (best ratio among fast-decompress codecs;
///   ZSTD decompresses at ~1500 MB/s vs Brotli's ~500 MB/s)
/// - Metadata: ZSTD L19
/// - Categorizers: enabled (FLAC, Rice++ for best ratio per file type)
/// - Chunks: 64 KB (fewer drops = fewer slab lookups during extract)
/// - Inline threshold: 8192 (more inline = fewer slab reads)
#[must_use]
pub fn max_read() -> WriteConfig {
    WriteConfig {
        defaults: Defaults {
            text_codec: "zstd".into(),
            binary_codec: "zstd".into(),
            metadata_codec: "zstd".into(),
            metadata_quality: 11,
            inline_threshold: 8192,
        },
        categorizers: crate::config::defaults::all_v0_1(),
        chunking: ChunkingConfig {
            name: "fastcdc".into(),
            avg_chunk_size: 65_536,
            min_chunk_size: 8192,
            max_chunk_size: 262_144,
        },
        tournament: TournamentConfig {
            codecs: vec!["store".into(), "lz4".into(), "zstd".into(), "brotli".into()],
            min_size_threshold: 256,
            skip_for_binary: false,
        },
        codec_tunables: CodecTunables {
            brotli: crate::config::BrotliTunables {
                quality: 11,
                window: 22,
            },
            lzma: crate::config::LzmaTunables {
                dict_size_mb: 64,
                use_optimal_parser: true,
                ..crate::config::LzmaTunables::default()
            },
            ..CodecTunables::default()
        },
        encryption: EncryptionConfig {
            aead: "chacha20-poly1305".into(),
            key_wrap: "x25519-hkdf".into(),
        },
        dictionaries: DictionaryConfig {
            enabled: true,
            min_class_size: 50,
            max_dict_size: 131_072,
            trainer: "frequency".into(),
        },
        mode: crate::config::ImageMode::ReadOnly,
        write_codec: "lz4".into(),
        turnover_threshold: 0,
    }
}

/// Maximum write profile — optimized for write-heavy workloads where
/// write latency matters more than ratio. Uses the fastest possible
/// compression (LZ4 at ~1 GB/s) and skips all categorization/tournament
/// overhead.
///
/// - Text/Binary/Metadata: LZ4 (fastest compress AND decompress)
/// - Categorizers: disabled (zero categorization overhead)
/// - Tournament: none (classify once, compress once)
/// - Chunks: 128 KB (minimal per-chunk overhead)
#[must_use]
pub fn max_write() -> WriteConfig {
    WriteConfig {
        defaults: Defaults {
            text_codec: "lz4".into(),
            binary_codec: "lz4".into(),
            metadata_codec: "lz4".into(),
            metadata_quality: 1,
            inline_threshold: 4096,
        },
        categorizers: vec![],
        chunking: ChunkingConfig {
            name: "fastcdc".into(),
            avg_chunk_size: 131_072,
            min_chunk_size: 16_384,
            max_chunk_size: 524_288,
        },
        tournament: TournamentConfig {
            codecs: vec!["store".into(), "lz4".into()],
            min_size_threshold: 0,
            skip_for_binary: true,
        },
        codec_tunables: CodecTunables::default(),
        encryption: EncryptionConfig {
            aead: "chacha20-poly1305".into(),
            key_wrap: "x25519-hkdf".into(),
        },
        dictionaries: DictionaryConfig {
            enabled: false,
            min_class_size: 0,
            max_dict_size: 0,
            trainer: "frequency".into(),
        },
        mode: crate::config::ImageMode::ReadOnly,
        write_codec: "lz4".into(),
        turnover_threshold: 0,
    }
}

/// Maximum write profile for RW images — optimized for write-heavy
/// live filesystems where write latency per operation matters most.
///
/// - Write codec: LZ4 (instant compress, minimal write latency)
/// - Turnover codec: ZSTD L12 (re-compaction with decent ratio)
/// - Mode: CopyOnWrite (fast updates, unreferenced blocks reclaimed)
/// - Chunks: 128 KB (minimal per-chunk overhead per write)
/// - Turnover threshold: 500 updates
#[must_use]
pub fn max_write_rw() -> WriteConfig {
    WriteConfig {
        defaults: Defaults {
            text_codec: "lz4".into(),
            binary_codec: "lz4".into(),
            metadata_codec: "lz4".into(),
            metadata_quality: 1,
            inline_threshold: 4096,
        },
        categorizers: vec![],
        chunking: ChunkingConfig {
            name: "fastcdc".into(),
            avg_chunk_size: 131_072,
            min_chunk_size: 16_384,
            max_chunk_size: 524_288,
        },
        tournament: TournamentConfig {
            codecs: vec!["store".into(), "lz4".into()],
            min_size_threshold: 0,
            skip_for_binary: true,
        },
        codec_tunables: CodecTunables::default(),
        mode: crate::config::ImageMode::ReadWrite(crate::config::RWMode::CopyOnWrite),
        write_codec: "lz4".into(),
        turnover_threshold: 500,
        encryption: EncryptionConfig {
            aead: "chacha20-poly1305".into(),
            key_wrap: "x25519-hkdf".into(),
        },
        dictionaries: DictionaryConfig {
            enabled: false,
            min_class_size: 0,
            max_dict_size: 0,
            trainer: "frequency".into(),
        },
    }
}

/// Maximum read profile for RW images — optimized for read-heavy
/// live filesystems where read throughput and integrity matter.
///
/// - Write codec: ZSTD L6 (good ratio, decent compress speed)
/// - Turnover codec: ZSTD L19 (best ratio for compaction)
/// - Mode: UpdateInPlace (full history for audit trail)
/// - Chunks: 64 KB (fewer drops to traverse during reads)
/// - Turnover threshold: 1000 updates
#[must_use]
pub fn max_read_rw() -> WriteConfig {
    WriteConfig {
        defaults: Defaults {
            text_codec: "zstd".into(),
            binary_codec: "zstd".into(),
            metadata_codec: "zstd".into(),
            metadata_quality: 6,
            inline_threshold: 8192,
        },
        categorizers: crate::config::defaults::all_v0_1(),
        chunking: ChunkingConfig {
            name: "fastcdc".into(),
            avg_chunk_size: 65_536,
            min_chunk_size: 8192,
            max_chunk_size: 262_144,
        },
        tournament: TournamentConfig {
            codecs: vec!["store".into(), "lz4".into(), "zstd".into(), "brotli".into()],
            min_size_threshold: 256,
            skip_for_binary: false,
        },
        codec_tunables: CodecTunables {
            brotli: crate::config::BrotliTunables {
                quality: 11,
                window: 22,
            },
            ..CodecTunables::default()
        },
        mode: crate::config::ImageMode::ReadWrite(crate::config::RWMode::UpdateInPlace),
        write_codec: "zstd".into(),
        turnover_threshold: 1000,
        encryption: EncryptionConfig {
            aead: "chacha20-poly1305".into(),
            key_wrap: "x25519-hkdf".into(),
        },
        dictionaries: DictionaryConfig {
            enabled: true,
            min_class_size: 50,
            max_dict_size: 131_072,
            trainer: "frequency".into(),
        },
    }
}

/// Balanced RW profile — general-purpose read-write image.
///
/// - Write codec: ZSTD L1 (fast, decent ratio per write)
/// - Turnover codec: Brotli q5 (good ratio compaction)
/// - Mode: UpdateInPlace
/// - Chunks: 16 KB
/// - Turnover threshold: 1000 updates
#[must_use]
pub fn balanced_rw() -> WriteConfig {
    WriteConfig {
        defaults: Defaults {
            text_codec: "zstd".into(),
            binary_codec: "lz4".into(),
            metadata_codec: "zstd".into(),
            metadata_quality: 3,
            inline_threshold: 4096,
        },
        categorizers: crate::config::defaults::all_v0_1(),
        chunking: ChunkingConfig {
            name: "fastcdc".into(),
            avg_chunk_size: 16_384,
            min_chunk_size: 2048,
            max_chunk_size: 65_536,
        },
        tournament: TournamentConfig {
            codecs: vec!["store".into(), "lz4".into(), "zstd".into()],
            min_size_threshold: 256,
            skip_for_binary: true,
        },
        codec_tunables: CodecTunables {
            brotli: crate::config::BrotliTunables {
                quality: 5,
                window: 22,
            },
            ..CodecTunables::default()
        },
        mode: crate::config::ImageMode::ReadWrite(crate::config::RWMode::UpdateInPlace),
        write_codec: "zstd".into(),
        turnover_threshold: 1000,
        encryption: EncryptionConfig {
            aead: "chacha20-poly1305".into(),
            key_wrap: "x25519-hkdf".into(),
        },
        dictionaries: DictionaryConfig {
            enabled: true,
            min_class_size: 100,
            max_dict_size: 65_536,
            trainer: "frequency".into(),
        },
    }
}

/// TOML-representable profile selector. Either a built-in name or
/// an inline custom profile.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ProfileSpec {
    /// Use a built-in profile by name.
    Preset(String),
    /// Define a custom profile inline.
    Custom(CustomProfile),
}

/// User-defined profile fields. Any field not specified inherits from
/// the `balanced` profile.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct CustomProfile {
    pub name: String,
    #[serde(default = "default_text")]
    pub text_codec: String,
    #[serde(default = "default_binary")]
    pub binary_codec: String,
    #[serde(default = "default_quality")]
    pub brotli_quality: u8,
    #[serde(default)]
    pub chunk_avg_size: u32,
    #[serde(default)]
    pub skip_tournament_for_binary: bool,
    #[serde(default = "default_true")]
    pub enable_flac: bool,
    #[serde(default = "default_true")]
    pub enable_ricepp: bool,
}

fn default_text() -> String {
    "brotli".into()
}
fn default_binary() -> String {
    "lz4".into()
}
fn default_quality() -> u8 {
    5
}
fn default_true() -> bool {
    true
}

/// Resolve a [`ProfileSpec`] into a concrete [`WriteConfig`].
pub fn resolve(spec: &ProfileSpec) -> Option<WriteConfig> {
    match spec {
        ProfileSpec::Preset(name) => select(name),
        ProfileSpec::Custom(custom) => {
            let mut config = balanced();
            if !custom.text_codec.is_empty() {
                config.defaults.text_codec = custom.text_codec.clone();
            }
            if !custom.binary_codec.is_empty() {
                config.defaults.binary_codec = custom.binary_codec.clone();
            }
            if custom.brotli_quality > 0 {
                config.codec_tunables.brotli.quality = custom.brotli_quality;
            }
            if custom.chunk_avg_size > 0 {
                config.chunking.avg_chunk_size = custom.chunk_avg_size;
            }
            config.tournament.skip_for_binary = custom.skip_tournament_for_binary;
            if !custom.enable_flac {
                config.categorizers.retain(|c| c.name != "pcm_audio");
            }
            if !custom.enable_ricepp {
                config.categorizers.retain(|c| c.name != "fits");
            }
            Some(config)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtins_resolve() {
        for name in [
            MAX_RATIO,
            MAX_SPEED,
            BALANCED,
            COMPETITIVE,
            MAX_READ,
            MAX_WRITE,
            MAX_WRITE_RW,
            MAX_READ_RW,
            BALANCED_RW,
        ] {
            let config = select(name).expect("profile exists");
            config.validate().expect("validates");
        }
    }

    #[test]
    fn competitive_uses_lz4_for_binary() {
        let config = competitive();
        assert_eq!(config.binary_codec_id().unwrap(), 0x01); // LZ4
    }

    #[test]
    fn competitive_uses_zstd_for_text() {
        let config = competitive();
        assert_eq!(config.text_codec_id().unwrap(), 0x02); // ZSTD
    }

    #[test]
    fn max_speed_disables_categorizers() {
        let config = max_speed();
        assert!(config.categorizers.is_empty());
    }

    #[test]
    fn max_ratio_enables_ppmd() {
        let config = max_ratio();
        assert_eq!(config.codec_tunables.ppmd7.memory_budget_mb, 256);
    }

    #[test]
    fn custom_profile_inherits_balanced() {
        let spec = ProfileSpec::Custom(CustomProfile {
            name: "test".into(),
            brotli_quality: 9,
            ..CustomProfile::default()
        });
        let config = resolve(&spec).expect("resolves");
        assert_eq!(config.codec_tunables.brotli.quality, 9);
        // Inherited from balanced
        assert_eq!(config.defaults.text_codec, "brotli");
    }
}
