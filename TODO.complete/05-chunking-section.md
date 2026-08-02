# 05: ChunkingConfig manifest section

## Status: IMPLEMENTED

## Scope

Add a new manifest section `chunking_config` that records the
FastCDC parameters used at write time. The reader can use this
to reconstruct the chunking or to verify the writer's choice.

## Why

FastCDC has tunable parameters (avg/min/max chunk size). Recording
them in the manifest makes the image self-describing and lets the
reader pre-size buffers.

## Design

### Manifest section

```rust
pub const CHUNKING_CONFIG_SECTION_VERSION: u8 = 1;

#[derive(Debug, Clone)]
pub struct ChunkingConfig {
    pub version: u8,
    pub avg_chunk_size: u32,
    pub min_chunk_size: u32,
    pub max_chunk_size: u32,
}
```

### Wire format

```
+--------------------+  1 byte: section_version = 1
| version            |
+--------------------+  4 bytes LE: avg_chunk_size
| avg_chunk_size     |
+--------------------+  4 bytes LE: min_chunk_size
| min_chunk_size     |
+--------------------+  4 bytes LE: max_chunk_size
| max_chunk_size     |
+--------------------+
```

### API

```rust
pub fn parse_chunking_config(cur: &mut ManifestCursor) -> Result<ChunkingConfig, CoreError>;
pub fn encode_chunking_config(cfg: &ChunkingConfig, out: &mut Vec<u8>);
```

## Implementation

1. New module `limnifs-core/src/chunking_config.rs`
2. Add type + parser + encoder
3. Wire into `ManifestRoot`
4. Specs: round-trip

## Related files

- New: `limnifs-core/src/chunking_config.rs`
- `limnifs-write/src/classifier.rs` (FastCDC)
