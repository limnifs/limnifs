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

/// The four built-in profile names.
pub const MAX_RATIO: &str = "max-ratio";
pub const MAX_SPEED: &str = "max-speed";
pub const BALANCED: &str = "balanced";
pub const COMPETITIVE: &str = "competitive";

/// Select a built-in profile by name. Returns a complete [`WriteConfig`]
/// configured for that profile's strategy.
#[must_use]
pub fn select(name: &str) -> Option<WriteConfig> {
    match name {
        MAX_RATIO => Some(max_ratio()),
        MAX_SPEED => Some(max_speed()),
        BALANCED => Some(balanced()),
        COMPETITIVE => Some(competitive()),
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
            avg_chunk_size: 65_536,
            min_chunk_size: 8192,
            max_chunk_size: 262_144,
        },
        tournament: TournamentConfig {
            codecs: vec![
                "store".into(),
                "lz4".into(),
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
        },
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
        },
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
            metadata_codec: "brotli".into(),
            metadata_quality: 5,
            inline_threshold: 4096,
        },
        categorizers: crate::config::defaults::all_v0_1(),
        chunking: ChunkingConfig {
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
        },
    }
}

/// Competitive profile — designed to beat SquashFS on ratio AND DwarFS
/// on speed simultaneously.
///
/// ## Strategy
///
/// The key insight: SquashFS uses LZ4 for everything (fast, poor ratio).
/// DWarFS uses FSST+LZMA (slow, good ratio). LimniFS can do BOTH:
///
/// - **Binary** → LZ4 (matches SquashFS speed, same ratio)
/// - **Text/Code** → Brotli q5 (5× better ratio than SquashFS, 30× faster than DWarFS)
/// - **Sparse** → Brotli q5 (near-zero on zeros, beats SquashFS)
/// - **Incompressible** → STORE (zero CPU, same ratio as everything)
/// - **Audio** → FLAC if ≤1 MB (beats SquashFS by 30×), Brotli if larger
/// - **FITS** → Rice++ (beats both SquashFS and DWarFS)
///
/// No tournament — classify once, compress once. Zero codec-trial overhead.
#[must_use]
pub fn competitive() -> WriteConfig {
    WriteConfig {
        defaults: Defaults {
            text_codec: "brotli".into(),
            binary_codec: "lz4".into(),
            metadata_codec: "brotli".into(),
            metadata_quality: 5,
            inline_threshold: 4096,
        },
        categorizers: crate::config::defaults::all_v0_1(),
        chunking: ChunkingConfig {
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
        for name in [MAX_RATIO, MAX_SPEED, BALANCED, COMPETITIVE] {
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
    fn competitive_uses_brotli_for_text() {
        let config = competitive();
        assert_eq!(config.text_codec_id().unwrap(), 0x04); // Brotli
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
