# 32 — DEFLATE/gzip codec (0x05)

- **Priority:** P1 (quick win — crate exists, production-ready)
- **Depends on:** 35 (codec registry refactor)
- **Estimated effort:** 1 hour

## Problem

DEFLATE is the most widely-deployed compression format in existence
(gzip, zlib, PNG, HTTP). A LimniFS image produced by an external
pipeline that pre-deflates drops needs a decode path. Conversely,
interoperability with gzip-compressed tarballs benefits from a
DEFLATE encode path.

## Goal

Reserve codec id **0x05** for DEFLATE (raw zlib stream). Encode via
`miniz_oxide` (pure Rust, full encoder + decoder). Decode via the
same crate.

## Wire format

| Id   | Name    | Encode | Decode | Notes |
|------|---------|--------|--------|-------|
| 0x05 | deflate | `miniz_oxide` | `miniz_oxide` | Pure Rust; gzip/zlib interop |

The compressed payload is a raw DEFLATE stream (RFC 1951). zlib
container (RFC 1950) and gzip container (RFC 1952) are NOT used —
LimniFS has its own framing via the drop record.

## Acceptance

- Round-trip at every compression level (0–9, plus
  `CompressionLevel::BestSpeed` / `Default` / `BestCompression`).
- `deflate_decodes_gzip_stream`: a gzip -9 corpus round-trips through
  LimniFS's deflate codec (container stripped upstream).
- Clippy clean, no `unsafe`, no GPL-3 transitive deps.
- CI green (linux + macOS).

## Implementation notes

- `miniz_oxide = "0.8"` (workspace dep).
- `compress_to_vec_zlib(input, level)` for encode;
  `inflate_bytes_zlib` for decode.
- Lower ratio than Brotli/ZSTD but universally interoperable.
