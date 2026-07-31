# 01 — Feature-gate ZSTD and XZ codecs for air-gapped baseline

- **Priority:** P0 (blocks air-gapped operation)
- **Depends on:** —
- **Estimated effort:** 2 hours

## Problem

Currently `zstd` and `xz2` (which wrap C libraries libzstd and liblzma)
are unconditionally compiled into limnifs-core. This breaks air-gapped
builds where C libraries are not available.

## Goal

Make ZSTD and XZ optional behind feature flags. The default build uses
only pure-Rust codecs (store + LZ4 via lz4_flex). Users opt in to ZSTD
or XZ via `--features zstd,xz`.

## Feature flags

```
limnifs-core features:
  default = []                    # store + lz4 only (pure Rust)
  zstd = ["dep:zstd"]             # adds ZSTD codec 0x02
  xz = ["dep:xz2"]                # adds XZ codec 0x03
```

The codec registry dispatches dynamically. At runtime, the writer
selects the best available codec per content class. If ZSTD is not
compiled in, Text/Code falls back to LZ4. If XZ is not compiled in,
Binary falls back to ZSTD or LZ4.

## Codec fallback matrix

| Available codecs | Text/Code | Binary | Compressed/Media |
|---|---|---|---|
| store + lz4 (default) | lz4 | lz4 | store |
| + zstd | zstd-9 | zstd-9 | store |
| + xz | zstd-9 | xz-6 | store |
| + zstd + xz | zstd-9 | xz-6 | store |

## Acceptance

- `cargo build -p limnifs-core` (no features) compiles without zstd/xz2
- `cargo build -p limnifs-core --features zstd` adds ZSTD
- `cargo build -p limnifs-core --features xz` adds XZ
- `cargo test --workspace` passes with each feature combination
- The slab reader gracefully rejects unknown codec ids (already does)
