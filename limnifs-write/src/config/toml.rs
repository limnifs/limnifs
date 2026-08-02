//! TOML load/serialise for `WriteConfig`.

use std::path::Path;

use crate::config::error::ConfigError;
use crate::config::WriteConfig;

impl WriteConfig {
    /// Load a config from a TOML file.
    /// # Errors
    /// Returns [`ConfigError::Io`] on read errors, [`ConfigError::Toml`]
    /// on parse errors, or [`ConfigError::InvalidValue`] on validation
    /// errors.
    pub fn from_toml(path: &Path) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&contents)?;
        config.validate()?;
        Ok(config)
    }

    /// Serialise to a TOML string.
    /// # Errors
    /// Returns [`ConfigError::TomlSer`] on serialisation errors.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Write a serialised config to a file.
    /// # Errors
    /// Returns [`ConfigError::Io`] on write errors or [`ConfigError::TomlSer`]
    /// on serialisation errors.
    pub fn write_to_toml(&self, path: &Path) -> Result<(), ConfigError> {
        let s = self.to_toml()?;
        std::fs::write(path, s)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_default() {
        let original = WriteConfig::default_v0_1();
        let s = original.to_toml().expect("serialise");
        let parsed: WriteConfig = toml::from_str(&s).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn parsed_with_all_fields() {
        let toml = r#"
[defaults]
text_codec = "brotli"
binary_codec = "lz4"
metadata_codec = "brotli"
metadata_quality = 5
inline_threshold = 4096

[[categorizer]]
name = "dna"
extensions = ["fasta", "fa"]
codec = "glza"
max_size = 524288
enabled = true

[chunking]
avg_chunk_size = 8192
min_chunk_size = 1024
max_chunk_size = 65536

[tournament]
codecs = ["store", "lz4", "zstd", "brotli"]
min_size_threshold = 256
skip_for_binary = true

[encryption]
aead = "chacha20-poly1305"
key_wrap = "x25519-hkdf"

[dictionaries]
enabled = true
min_class_size = 100
max_dict_size = 65536
"#;
        let config: WriteConfig = toml::from_str(toml).expect("parse");
        config.validate().expect("validate");
        assert_eq!(config.categorizers.len(), 1);
        assert_eq!(config.categorizers[0].name, "dna");
        assert_eq!(config.categorizers[0].max_size, Some(524_288));
    }

    #[test]
    fn defaults_resolved_correctly() {
        let config = WriteConfig::default_v0_1();
        assert_eq!(config.text_codec_id().unwrap(), 0x04);
        assert_eq!(config.binary_codec_id().unwrap(), 0x01);
    }
}
