//! User-facing write configuration.
//!
//! [`WriteConfig`] captures all user-tunable writer settings in one
//! place. The configuration is model-driven: each sub-config is a
//! distinct type that owns its own validation, serialization, and
//! defaults. This eliminates the v0.1 pattern of hardcoded
//! constants scattered across `limnifs-write/src/lib.rs`.
//!
//! ## Architecture
//!
//! ```text
//! WriteConfig (top-level)
//!   ├── Defaults           (codecs, qualities, inline threshold)
//!   ├── CategorizerConfig[] (extension/magic → codec routing)
//!   ├── ChunkingConfig      (FastCDC parameters)
//!   ├── TournamentConfig   (which codecs + min sizes)
//!   ├── EncryptionConfig    (AEAD + key wrap)
//!   └── DictionaryConfig    (ZSTD dictionary training)
//! ```
//!
//! ## OCP
//!
//! Adding a new sub-config = adding a new struct + wiring into
//! [`WriteConfig::default_v0_1`] + adding a TOML section. No
//! existing code changes.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

pub mod defaults;
pub mod error;
pub mod toml;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::error::ConfigError;

/// Default codec for text/code/sparse content.
/// Matches v0.1 behavior: Brotli.
pub const DEFAULT_TEXT_CODEC: &str = "brotli";
/// Default codec for binary content.
/// Matches v0.1 behavior: LZ4.
pub const DEFAULT_BINARY_CODEC: &str = "lz4";
/// Default codec for the metadata blob.
/// Matches v0.1 behavior: Brotli.
pub const DEFAULT_METADATA_CODEC: &str = "brotli";
/// Default Brotli quality for small metadata blobs.
pub const DEFAULT_METADATA_QUALITY: u8 = 5;
/// Default inline-data threshold (bytes).
pub const DEFAULT_INLINE_THRESHOLD: u16 = 4096;
/// Default FastCDC average chunk size.
pub const DEFAULT_AVG_CHUNK_SIZE: u32 = 8192;
/// Default FastCDC minimum chunk size.
pub const DEFAULT_MIN_CHUNK_SIZE: u32 = 1024;
/// Default FastCDC maximum chunk size.
pub const DEFAULT_MAX_CHUNK_SIZE: u32 = 65_536;
/// Default minimum size for the tournament to try a codec.
pub const DEFAULT_TOURNAMENT_MIN_SIZE: u32 = 256;
/// Default: skip tournament for binary class.
pub const DEFAULT_TOURNAMENT_SKIP_BINARY: bool = true;
/// Default AEAD algorithm.
pub const DEFAULT_AEAD: &str = "chacha20-poly1305";
/// Default key wrap algorithm.
pub const DEFAULT_KEY_WRAP: &str = "x25519-hkdf";
/// Default: enable dictionary training.
pub const DEFAULT_DICT_ENABLED: bool = true;
/// Default minimum drops per class to train a dict.
pub const DEFAULT_DICT_MIN_CLASS_SIZE: u32 = 100;
/// Default maximum dictionary size in bytes.
pub const DEFAULT_DICT_MAX_SIZE: u32 = 65_536;

/// Top-level write configuration. All fields are public so the
/// TOML loader can construct values directly; runtime validation
/// lives in [`WriteConfig::validate`].
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WriteConfig {
    /// Default codec selection + inline threshold.
    pub defaults: Defaults,
    /// File categorizer rules (extension/magic → codec).
    #[serde(default, rename = "categorizer")]
    pub categorizers: Vec<CategorizerConfig>,
    /// FastCDC chunking parameters.
    pub chunking: ChunkingConfig,
    /// Compression tournament settings.
    pub tournament: TournamentConfig,
    /// Encryption configuration.
    pub encryption: EncryptionConfig,
    /// ZSTD dictionary configuration.
    pub dictionaries: DictionaryConfig,
}

/// Default codec + quality settings.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Defaults {
    pub text_codec: String,
    pub binary_codec: String,
    pub metadata_codec: String,
    pub metadata_quality: u8,
    /// Inline data threshold (bytes).
    pub inline_threshold: u16,
}

/// One file categorizer rule.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CategorizerConfig {
    /// Human-readable name (e.g. "dna", "json").
    pub name: String,
    /// File extensions that trigger this rule (lowercase, no dot).
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Magic bytes at offset 0 that trigger this rule.
    #[serde(default)]
    pub magic_bytes: Vec<u8>,
    /// Codec identifier (string name or numeric id).
    pub codec: String,
    /// Maximum file size to apply this rule to.
    #[serde(default)]
    pub max_size: Option<u32>,
    /// Whether this rule is active.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// FastCDC parameters.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChunkingConfig {
    pub avg_chunk_size: u32,
    pub min_chunk_size: u32,
    pub max_chunk_size: u32,
}

/// Compression tournament settings.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TournamentConfig {
    /// Codecs to try in the tournament (ordered).
    pub codecs: Vec<String>,
    /// Minimum chunk size for the tournament to try a codec.
    pub min_size_threshold: u32,
    /// Skip tournament for binary class (use binary_codec directly).
    pub skip_for_binary: bool,
}

/// Encryption configuration.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct EncryptionConfig {
    /// AEAD algorithm name.
    pub aead: String,
    /// Key wrap algorithm name.
    pub key_wrap: String,
}

/// ZSTD dictionary training configuration.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DictionaryConfig {
    pub enabled: bool,
    pub min_class_size: u32,
    pub max_dict_size: u32,
}

impl WriteConfig {
    /// Create the v0.1-compatible default configuration.
    /// All fields match the behavior of `limnifs-write` before
    /// this config was introduced.
    #[must_use]
    pub fn default_v0_1() -> Self {
        Self {
            defaults: Defaults {
                text_codec: DEFAULT_TEXT_CODEC.to_string(),
                binary_codec: DEFAULT_BINARY_CODEC.to_string(),
                metadata_codec: DEFAULT_METADATA_CODEC.to_string(),
                metadata_quality: DEFAULT_METADATA_QUALITY,
                inline_threshold: DEFAULT_INLINE_THRESHOLD,
            },
            categorizers: Vec::new(),
            chunking: ChunkingConfig {
                avg_chunk_size: DEFAULT_AVG_CHUNK_SIZE,
                min_chunk_size: DEFAULT_MIN_CHUNK_SIZE,
                max_chunk_size: DEFAULT_MAX_CHUNK_SIZE,
            },
            tournament: TournamentConfig {
                codecs: vec![
                    "store".to_string(),
                    "lz4".to_string(),
                    "zstd".to_string(),
                    "brotli".to_string(),
                ],
                min_size_threshold: DEFAULT_TOURNAMENT_MIN_SIZE,
                skip_for_binary: DEFAULT_TOURNAMENT_SKIP_BINARY,
            },
            encryption: EncryptionConfig {
                aead: DEFAULT_AEAD.to_string(),
                key_wrap: DEFAULT_KEY_WRAP.to_string(),
            },
            dictionaries: DictionaryConfig {
                enabled: DEFAULT_DICT_ENABLED,
                min_class_size: DEFAULT_DICT_MIN_CLASS_SIZE,
                max_dict_size: DEFAULT_DICT_MAX_SIZE,
            },
        }
    }

    /// Validate field relationships and range constraints.
    /// # Errors
    /// Returns a [`ConfigError`] on any invalid value.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.chunking.min_chunk_size > self.chunking.avg_chunk_size {
            return Err(ConfigError::InvalidValue {
                field: "chunking.min_chunk_size".into(),
                reason: format!(
                    "min_chunk_size ({}) > avg_chunk_size ({})",
                    self.chunking.min_chunk_size, self.chunking.avg_chunk_size
                ),
            });
        }
        if self.chunking.avg_chunk_size > self.chunking.max_chunk_size {
            return Err(ConfigError::InvalidValue {
                field: "chunking.avg_chunk_size".into(),
                reason: format!(
                    "avg_chunk_size ({}) > max_chunk_size ({})",
                    self.chunking.avg_chunk_size, self.chunking.max_chunk_size
                ),
            });
        }
        if self.defaults.metadata_quality < 1 || self.defaults.metadata_quality > 11 {
            return Err(ConfigError::InvalidValue {
                field: "defaults.metadata_quality".into(),
                reason: format!(
                    "metadata_quality ({}) out of range 1..=11",
                    self.defaults.metadata_quality
                ),
            });
        }
        // Validate unique categorizer names.
        let mut names_seen: BTreeMap<&str, ()> = BTreeMap::new();
        for rule in &self.categorizers {
            if !names_seen
                .insert(rule.name.as_str(), ())
                .map_or(true, |_| false)
            {
                return Err(ConfigError::DuplicateCategorizer(rule.name.clone()));
            }
        }
        Ok(())
    }

    /// Build the codec registry to use for this config.
    /// Maps codec names to numeric ids.
    pub fn codec_registry(&self) -> Result<CodecRegistry, ConfigError> {
        let mut registry = CodecRegistry::default();
        registry.insert("store", 0x00);
        registry.insert("lz4", 0x01);
        registry.insert("zstd", 0x02);
        registry.insert("xz", 0x03);
        registry.insert("brotli", 0x04);
        registry.insert("deflate", 0x05);
        registry.insert("snappy", 0x06);
        registry.insert("flac", 0x07);
        registry.insert("ricepp", 0x08);
        registry.insert("fsst+brotli", 0x09);
        registry.insert("shuffle+lz4", 0x0A);
        registry.insert("zpaq", 0x0B);
        registry.insert("ppmd", 0x0C);
        registry.insert("glza", 0x0D);
        registry.insert("shuffle+zstd", 0x0E);
        registry.insert("bitshuffle+lz4", 0x0F);
        registry.insert("bzip2", 0x10);
        registry.insert("deflate64", 0x11);

        if !registry.contains_name(&self.defaults.text_codec) {
            return Err(ConfigError::UnknownCodec(self.defaults.text_codec.clone()));
        }
        if !registry.contains_name(&self.defaults.binary_codec) {
            return Err(ConfigError::UnknownCodec(
                self.defaults.binary_codec.clone(),
            ));
        }
        if !registry.contains_name(&self.defaults.metadata_codec) {
            return Err(ConfigError::UnknownCodec(
                self.defaults.metadata_codec.clone(),
            ));
        }
        for rule in &self.categorizers {
            if !registry.contains_name(&rule.codec) {
                return Err(ConfigError::UnknownCodec(rule.codec.clone()));
            }
        }
        for codec in &self.tournament.codecs {
            if !registry.contains_name(codec) {
                return Err(ConfigError::UnknownCodec(codec.clone()));
            }
        }
        Ok(registry)
    }

    /// Resolve the default text codec id.
    /// # Errors
    /// Returns [`ConfigError`] if the codec name is unknown.
    pub fn text_codec_id(&self) -> Result<u8, ConfigError> {
        let registry = self.codec_registry()?;
        Ok(registry
            .lookup_by_name(&self.defaults.text_codec)
            .unwrap_or(0x04))
    }

    /// Resolve the default binary codec id.
    /// # Errors
    /// Returns [`ConfigError`] if the codec name is unknown.
    pub fn binary_codec_id(&self) -> Result<u8, ConfigError> {
        let registry = self.codec_registry()?;
        Ok(registry
            .lookup_by_name(&self.defaults.binary_codec)
            .unwrap_or(0x01))
    }

    /// Resolve the default metadata codec id.
    /// # Errors
    /// Returns [`ConfigError`] if the codec name is unknown.
    pub fn metadata_codec_id(&self) -> Result<u8, ConfigError> {
        let registry = self.codec_registry()?;
        Ok(registry
            .lookup_by_name(&self.defaults.metadata_codec)
            .unwrap_or(0x04))
    }
}

/// Bidirectional map between codec names and numeric ids.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CodecRegistry {
    by_name: BTreeMap<String, u8>,
    by_id: BTreeMap<u8, String>,
}

impl CodecRegistry {
    /// Insert a new (name, id) mapping.
    pub fn insert(&mut self, name: &str, id: u8) {
        self.by_name.insert(name.to_string(), id);
        self.by_id.insert(id, name.to_string());
    }

    /// Look up a codec id by name.
    #[must_use]
    pub fn lookup_by_name(&self, name: &str) -> Option<u8> {
        self.by_name.get(name).copied()
    }

    /// Look up a codec name by id.
    #[must_use]
    pub fn lookup_by_id(&self, id: u8) -> Option<&str> {
        self.by_id.get(&id).map(String::as_str)
    }

    /// Returns true if the name is registered.
    #[must_use]
    pub fn contains_name(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_v0_1_validates() {
        let config = WriteConfig::default_v0_1();
        config.validate().expect("v0.1 default should validate");
    }

    #[test]
    fn default_v0_1_codec_ids() {
        let config = WriteConfig::default_v0_1();
        assert_eq!(config.text_codec_id().unwrap(), 0x04); // brotli
        assert_eq!(config.binary_codec_id().unwrap(), 0x01); // lz4
        assert_eq!(config.metadata_codec_id().unwrap(), 0x04); // brotli
    }

    #[test]
    fn rejects_invalid_chunking() {
        let mut config = WriteConfig::default_v0_1();
        config.chunking.min_chunk_size = 16_000;
        config.chunking.avg_chunk_size = 8_000;
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_quality() {
        let mut config = WriteConfig::default_v0_1();
        config.defaults.metadata_quality = 12;
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_duplicate_categorizer_names() {
        let mut config = WriteConfig::default_v0_1();
        config.categorizers.push(CategorizerConfig {
            name: "dna".into(),
            extensions: vec!["fasta".into()],
            magic_bytes: vec![],
            codec: "glza".into(),
            max_size: None,
            enabled: true,
        });
        config.categorizers.push(CategorizerConfig {
            name: "dna".into(),
            extensions: vec!["fa".into()],
            magic_bytes: vec![],
            codec: "glza".into(),
            max_size: None,
            enabled: true,
        });
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_unknown_codec() {
        let mut config = WriteConfig::default_v0_1();
        config.defaults.text_codec = "does-not-exist".into();
        assert!(config.codec_registry().is_err());
    }
}
