# 01: WriteConfig — user-facing configuration

## Status: IMPLEMENTED

## Scope

Introduce a `WriteConfig` struct that captures all user-tunable writer
settings in one place. This is the foundation for ALL other v0.2
features: categorizers, AEAD, compression tournament, chunking,
inline thresholds, dictionaries.

## Why

Today the writer uses hardcoded defaults scattered across `limnifs-write/src/lib.rs`:
- `best_compressible_codec()` returns Brotli
- `best_binary_codec()` returns LZ4
- Metadata codec is hardcoded Brotli
- FastCDC defaults are implicit
- Inline threshold is implicit
- No way to add custom file categorizers

Users on different workloads (genomics, scientific data, game assets)
need different defaults. Options belong in a config, not in source.

## Design

### Struct

```rust
#[derive(Debug, Clone)]
pub struct WriteConfig {
    /// Default codec for text/code/sparse content.
    pub default_text_codec: u8,
    /// Default codec for binary content.
    pub default_binary_codec: u8,
    /// Codec for the metadata blob (inode table, dir nodes).
    pub default_metadata_codec: u8,
    /// Quality level for the metadata codec (1–11 for Brotli).
    pub default_metadata_quality: u8,
    /// File categorizers (extension/magic → codec routing).
    pub categorizers: Vec<CategorizerConfig>,
    /// FastCDC chunking parameters.
    pub chunking: ChunkingConfig,
    /// Compression tournament settings.
    pub tournament: CompressionTournamentConfig,
    /// Inline data threshold (bytes).
    pub inline_threshold: u16,
    /// Encryption configuration.
    pub encryption: EncryptionConfig,
    /// ZSTD dictionary configuration.
    pub dictionaries: DictionaryConfig,
}

impl WriteConfig {
    /// Default config matching v0.1 behaviour.
    pub fn default_v0_1() -> Self;
    /// Load from a TOML file.
    pub fn from_toml(path: &Path) -> Result<Self, ConfigError>;
    /// Serialize to TOML.
    pub fn to_toml(&self) -> Result<String, ConfigError>;
}
```

### Sub-configs

```rust
#[derive(Debug, Clone)]
pub struct CategorizerConfig {
    pub name: String,
    pub extensions: Vec<String>,
    pub magic_bytes: Vec<u8>,
    pub codec: u8,
    pub max_size: Option<u32>,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct ChunkingConfig {
    pub avg_chunk_size: u32,
    pub min_chunk_size: u32,
    pub max_chunk_size: u32,
}

#[derive(Debug, Clone)]
pub struct CompressionTournamentConfig {
    pub codecs: Vec<u8>,
    pub min_size_threshold: u32,
    pub skip_for_binary: bool,
}

#[derive(Debug, Clone)]
pub struct EncryptionConfig {
    pub aead: u8,         // 0x01 = ChaCha20-Poly1305, 0x02 = AES-256-GCM, 0x03 = AES-256-OCB
    pub key_wrap: u8,     // 0x01 = X25519+HKDF
}

#[derive(Debug, Clone)]
pub struct DictionaryConfig {
    pub enabled: bool,
    pub min_class_size: u32,  // Train dict only if class has N+ drops
    pub max_dict_size: u32,
}
```

### TOML shape

```toml
[defaults]
text_codec = "brotli"
binary_codec = "lz4"
metadata_codec = "brotli"
metadata_quality = 5
inline_threshold = 4096

[[categorizer]]
name = "dna"
extensions = ["fasta", "fa", "fna"]
codec = "glza"
max_size = 524288  # 512 KB
enabled = true

[[categorizer]]
name = "json"
extensions = ["json", "jsonl"]
codec = "fsst+brotli"
max_size = 65536
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
```

### Writer API

```rust
/// Create an image with default v0.1 config.
pub fn write_directory(root: &Path) -> Result<WriteArtifact, WriteError>;

/// Create an image with custom config.
pub fn write_directory_with_config(
    root: &Path,
    config: &WriteConfig,
) -> Result<WriteArtifact, WriteError>;
```

The default `write_directory()` becomes a thin wrapper around `write_directory_with_config()`
with `WriteConfig::default_v0_1()`.

## Implementation

1. New crate `limnifs-config` (or module in `limnifs-write/src/config.rs`)
2. Add `toml` + `serde` + `thiserror` dependencies
3. Build `WriteConfig` with all sub-configs
4. Map `WriteConfig` → existing `WriteContext`
5. Refactor `write_directory()` to delegate to `write_directory_with_config()`
6. Specs: TOML round-trip, defaults, validation

## Related files

- `limnifs-write/src/lib.rs` (writer entry point)
- `limnifs-write/src/file_categorizer/` (categorizer registry)
- `limnifs-core/src/codec/mod.rs` (codec registry)
- New: `limnifs-write/src/config.rs` (WriteConfig)
- New: `limnifs-write/src/config/defaults.rs` (v0.1 defaults)
- New: `limnifs-write/src/config/toml.rs` (TOML load/save)
- New: `limnifs-write/src/config/error.rs` (ConfigError)
