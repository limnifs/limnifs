//! Configuration errors.

#![allow(clippy::module_name_repetitions)]

use std::io;

/// Errors that can occur when loading or validating a `WriteConfig`.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// I/O error when reading the config file.
    #[error("I/O error reading config: {0}")]
    Io(#[from] io::Error),

    /// TOML parse error.
    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    /// TOML serialisation error.
    #[error("TOML serialise error: {0}")]
    TomlSer(#[from] toml::ser::Error),

    /// A field value is out of range or violates a relation.
    #[error("invalid value for {field}: {reason}")]
    InvalidValue {
        /// Field name (e.g. `"chunking.min_chunk_size"`).
        field: String,
        /// Why it's invalid.
        reason: String,
    },

    /// A categorizer name appears more than once.
    #[error("duplicate categorizer name: {0}")]
    DuplicateCategorizer(String),

    /// A codec name is not in the registry.
    #[error("unknown codec: {0}")]
    UnknownCodec(String),
}
