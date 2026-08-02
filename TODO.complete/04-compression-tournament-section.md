# 04: CompressionTournamentConfig manifest section

## Status: IMPLEMENTED

## Scope

Add a new manifest section `compression_tournament_config` that
records which codecs the writer tried in the tournament and the
min-size threshold. The reader can use this to optimize expected
codec patterns.

## Why

Today the writer's compression tournament is hardcoded (try
`best_compressible_codec` for text, `best_binary_codec` for
binary, STORE for everything else). The reader has no way to know
which codecs might appear in the slab. Recording the tournament
config makes the image self-describing.

## Design

### Manifest section

```rust
pub const COMPRESSION_TOURNAMENT_SECTION_VERSION: u8 = 1;

#[derive(Debug, Clone)]
pub struct CompressionTournamentConfig {
    pub version: u8,
    pub codecs: Vec<u8>,
    pub min_size_threshold: u32,
    pub skip_for_binary: bool,
}
```

### Wire format

```
+--------------------+  1 byte: section_version = 1
| version            |
+--------------------+  1 byte: codec_count
| codec_count        |
+--------------------+  codec_count bytes: each codec_id
| codec_ids[]        |
+--------------------+  4 bytes LE: min_size_threshold
| min_size_threshold |
+--------------------+  1 byte: flags (bit 0 = skip_for_binary)
| flags              |
+--------------------+
```

### API

```rust
pub fn parse_compression_tournament_config(cur: &mut ManifestCursor) -> Result<CompressionTournamentConfig, CoreError>;
pub fn encode_compression_tournament_config(cfg: &CompressionTournamentConfig, out: &mut Vec<u8>);
```

## Implementation

1. New module `limnifs-core/src/compression_tournament_config.rs`
2. Add type + parser + encoder
3. Wire into `ManifestRoot`
4. Add `CompressionTournamentConfig::to_manifest_section()` conversion
5. Specs: round-trip

## Related files

- `limnifs-core/src/codec/mod.rs` (codec registry)
- New: `limnifs-core/src/compression_tournament_config.rs`
