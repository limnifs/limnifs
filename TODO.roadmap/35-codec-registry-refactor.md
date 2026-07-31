# 35 — Codec registry refactor (OCP compliance)

- **Priority:** P0 (blocks 31, 32, 33, 34 — every new codec)
- **Depends on:** PR #108 (pure-Rust codecs migration)
- **Estimated effort:** half a day

## Problem

The codec dispatch in `limnifs-core/src/codec.rs` is a `match`
statement over `codec_id`. Adding a new codec (Brotli, DEFLATE, or
the future full ZSTD/LZMA encoders) requires editing the match arms
in `compress()` and `decompress()` — violating the open/closed
principle that CAMPAIGN.md §1 mandates:

> OCP (open/closed): Every variation point is a *registry*, never a
> switch statement.

The same rule applies to AEAD, locator, classifier, and feature-flag
registries that already exist. Codec is the holdout.

## Goal

Replace the `match codec_id` with a `Codec` trait + a process-wide
registry. New codecs register themselves; existing code never changes.

## Design

```rust
pub trait Codec: Send + Sync {
    fn id(&self) -> u8;
    fn name(&self) -> &'static str;
    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError>;
    fn decompress(&self, compressed: &[u8], expected_len: u32)
        -> Result<Vec<u8>, CoreError>;
}

// In codec/registry.rs:
pub struct CodecRegistry { codecs: Vec<Box<dyn Codec>> }
impl CodecRegistry {
    pub fn register(&mut self, codec: Box<dyn Codec>);
    pub fn compress(&self, id: u8, plaintext: &[u8]) -> Result<Vec<u8>, CoreError>;
    pub fn decompress(&self, id: u8, compressed: &[u8], expected_len: u32)
        -> Result<Vec<u8>, CoreError>;
}

// Default codec set (registered in `CodecRegistry::default`):
//   0x00 store, 0x01 lz4, 0x02 zstd, 0x03 xz (decode-only).
//   0x04 brotli and 0x05 deflate land via 31 and 32.
```

The existing free functions `compress(codec_id, …)` and
`decompress(codec_id, …)` become thin wrappers around a default
registry, preserving the public API.

## Acceptance

- `compress()` and `decompress()` behaviour unchanged — all existing
  tests pass without modification.
- `CODEC_REGISTRY.register(Box::new(BrotliCodec::default()))` in a
  unit test successfully compresses with Brotli without editing
  `compress()`'s match arm.
- Clippy clean, no `unsafe`, no GPL-3 transitive deps.
- CI green (linux + macOS).

## Implementation notes

- Registry is a `Vec` (codec ids are sparse u8, typically < 256).
- Lookup is `O(n)` where n is registered codec count (typically 5–6).
  A `[Option<&dyn Codec>; 256]` direct-indexed table is the zero-
  cost alternative — choose it if profiling shows the registry lookup
  is hot (it isn't: compress/decompress dominate).
- Keep `CODEC_STORE` / `CODEC_LZ4` / `CODEC_ZSTD` / `CODEC_XZ` as
  public constants for ergonomic dispatch in the writer.
- Move `compress_lz4_with_size`, `compress_zstd`, `compress_xz`
  helpers to their respective codec implementations.
