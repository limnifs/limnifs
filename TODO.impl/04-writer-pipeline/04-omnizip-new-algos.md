# 04 — omnizip new algorithms investigation (2026-08-03)

- **Status:** in_progress
- **Phase:** 1–2 (mixed)
- **Depends on:** 04-specialized-codecs, 04-ppmd-quality-wiring
- **Design refs:** 2026-throughput-roadmap.md §3
- **Priority:** P0

## Goal

Survey the published `omnizip` 0.13.1 workspace (17 crates) and
identify algorithms we don't wire today but should. Each finding
is scored for impact, risk, and effort.

## What we already wire (baseline)

| Omnizip crate | limnifs-core codec id | wired? |
|---|---|---|
| omnizip-snappy | `CODEC_SNAPPY` 0x06 | yes |
| omnizip-zstd | `CODEC_ZSTD` 0x02 | yes |
| omnizip-lzma | `CODEC_XZ` 0x03 | **decode only** — encoder stubbed in our code |
| omnizip-deflate | `CODEC_DEFLATE` 0x05 | yes |
| omnizip-brotli | `CODEC_BROTLI` 0x04 | yes |
| omnizip-lz4 | `CODEC_LZ4` 0x01 | yes (via `lz4_flex` directly, not the omnizip wrapper) |
| omnizip-flac | `CODEC_FLAC` 0x07 | yes |
| omnizip-fsst | `CODEC_FSST_BROTLI` 0x09 (composite) | yes |
| omnizip-ricepp | `CODEC_RICEPP` 0x08 | yes |
| omnizip-glza | `CODEC_GLZA` 0x0D | yes |
| omnizip-ppmd | `CODEC_PPMD` 0x0C, `CODEC_PPMD8` 0x12 | yes (tunables wired this cycle) |
| omnizip-zpaq | `CODEC_ZPAQ` 0x0B | yes |
| omnizip-blosc | `CODEC_BLOSC2_SHUFFLE_LZ4` 0x0A (one variant) | partial |
| omnizip-bzip2 | `CODEC_BZIP2` 0x10 | yes |
| omnizip-deflate64 | `CODEC_DEFLATE64` 0x11 | yes |
| omnizip-filters | shuffle only (via shuffle_lz4 etc.) | **delta + 7 BCJ filters not wired** |
| omnizip-codecs | (shared trait; not a codec itself) | n/a |

## Findings

### Finding 1 — LZMA encoder is real, we treat it as decode-only (P0, this cycle)

`omnizip-lzma::xz_compress` actually compresses (verified:
9 KB repetitive text → 4.7 KB XZ container). Our `xz::XzCodec::compress`
returns `UnsupportedFeature` and routes to ZSTD. Wire the encoder.

- **Impact**: LZMA beats ZSTD L6 on text by ~10-15% ratio, ~3×
  slower encode. Useful for `max-ratio` profile.
- **Risk**: low. Decoder already works; encoder is pure Rust.
- **Effort**: ~30 lines in `limnifs-core/src/codec/xz.rs`.

### Finding 2 — LZ4 HC is a stub in omnizip-lz4 0.13.1 (do NOT wire)

The `Lz4HcCodec::compress` body is identical to `Lz4FastCodec::compress`
— both call `lz4_flex::compress_prepend_size`. The HC match finder
isn't actually invoked; ratio is identical to fast LZ4. Confirmed
by direct test.

- **Action**: skip. File an upstream issue against omnizip-lz4.
- **When omnizip ships a real HC**: assign codec id `0x13`, wire
  via omnizip-lzma-style encoder.

### Finding 3 — BCJ filters not wired; major win for executable binaries (P0, this cycle)

`omnizip-filters` exposes 7 BCJ filters (Branch/Call/Jump) that
convert relative call/branch addresses in executable code to
absolute values, dramatically improving downstream codec ratio
on ELF/PE/Mach-O. Verified the filter trait is `Filter::{encode,
decode}` with byte-exact inverse.

Typical win on a Linux kernel image:
- Raw LZ4 ratio: ~70%
- BCJ-x86 + LZ4 ratio: ~45-50%

We have the codec id space (currently uses 0x00..=0x12). Assign:

| Composite codec | id | pipeline |
|---|---|---|
| `BCJ_X86_LZ4` | 0x20 | bcj_x86 → lz4 |
| `BCJ_X86_ZSTD` | 0x21 | bcj_x86 → zstd |
| `BCJ_X86_LZMA` | 0x22 | bcj_x86 → lzma (when finding 1 lands) |
| `BCJ_ARM64_LZ4` | 0x23 | bcj_arm64 → lz4 |

(Bcj2 splits into 5 streams; defer — it doesn't fit the
single-stream codec shape.)

- **Impact**: high for `binary` content class.
- **Risk**: low. Filter is its own inverse.
- **Effort**: ~80 lines for the composite codec + register.

### Finding 4 — Delta filter useful but covered by shuffle_lz4 (defer)

`omnizip-filters::DeltaFilter` is a special case of byte-shuffle
for distance=1. Our `shuffle_lz4` codec at `item_size=1` is
equivalent. Don't add a separate delta codec.

### Finding 5 — Libdeflate slot is reserved, not implemented (skip)

`omnizip-codecs::CodecId::LIBDEFLATE = 0x000B` is reserved but
there's no `omnizip-libdeflate` crate. Skip until published.

## Acceptance

- [ ] This doc exists with the survey above.
- [ ] `xz::XzCodec::compress` actually compresses via
      `omnizip_lzma::xz_compress`.
- [ ] `BCJ_X86_LZ4` composite codec exists and round-trips in tests.
- [ ] A test on a synthetic "executable-like" fixture (lots of
      relative call addresses) shows BCJ+LZ4 beats plain LZ4.
- [ ] Categorizer integration (routing `binary` class to BCJ) is
      filed as `04-bcj-categorizer-routing.md` (P1) — out of
      scope this cycle.

## Filed follow-ups

- `04-bcj-categorizer-routing.md` (P1) — auto-route ELF/PE to BCJ.
- `04-lz4-hc-when-ready.md` (P2) — wire LZ4 HC when omnizip ships
  a real impl.
