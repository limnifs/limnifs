# 31 — Brotli codec (0x04)

- **Priority:** P1 (quick win — crate exists, production-ready)
- **Depends on:** 35 (codec registry refactor)
- **Estimated effort:** 2 hours

## Problem

LimniFS has no high-ratio pure-Rust codec. Brotli beats both ZSTD and
LZMA on text and web content at high levels, and the `brotli` crate
(by Daniel Reiter Horn, the format's original author) is 100% pure
Rust with a full encoder (levels 0–11) and decoder. Adding it is a
one-line dependency and a new codec id.

## Goal

Reserve codec id **0x04** for Brotli. Encode at configurable level
(0–11); decode at any level. Route the "best ratio" content class to
Brotli level 11 (deepening stage); route the "fast" class to ZSTD
level 1 or LZ4.

## Wire format

| Id   | Name   | Encode                      | Decode | Notes |
|------|--------|-----------------------------|--------|-------|
| 0x04 | brotli | `brotli` crate, levels 0–11 | `brotli` crate | Pure Rust; best ratio on text/web |

The compressed payload is a raw Brotli stream (no container). The
drop record's `plaintext_len` field provides the integrity check on
decode.

## Acceptance

- `cargo test -p limnifs-core codec::brotli` round-trips at every level.
- `brotli_compresses_better_than_zstd_on_text`: Brotli L11 < ZSTD L1
  on a 1 MB English-text corpus.
- `brotli_compresses_binary_data`: Brotli L11 < LZ4 on a 1 MB binary
  corpus.
- Clippy clean, no `unsafe`, no GPL-3 transitive deps.
- CI green (linux + macOS).

## Implementation notes

- `brotli = "3.4"` (workspace dep).
- `BrotliEncoder::new(writer).compress(reader, &params)` for encode;
  `brotli::Decompressor` for decode.
- Level mapping: LimniFS level 0 → Brotli quality 0; level 11 →
  quality 11 (1:1 with the reference encoder).
