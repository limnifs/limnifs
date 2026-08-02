---
Component: 04-writer-pipeline
Task: 04-specialized-codecs
Status: done (ricepp + FLAC with LPC both shipped)
Depends on: 04-classifier-seine, 04-file-level-categorization
Unblocks: —
Source: docs/dwarfs-multicodec-investigation.md (Tier 2)
Rice++ landed: limnifs-core/src/codec/ricepp.rs (id 0x08).
  FITS benchmark: 32.08% (beats DwarFS 90%, SquashFS 90%).
FLAC landed: limnifs-core/src/codec/flac.rs (id 0x07).
  Full encoder (omnizip-flac 0.10): CONSTANT/VERBATIM/FIXED/LPC +
  Rice residuals with optimal-k. Subframe type selected by bit-cost
  estimation per block. FLAC_ENABLED=true.
---

# 04-specialized-codecs — FLAC (audio) + Rice++ (FITS images)

## Problem

General-purpose codecs (Brotli, LZ4, ZSTD) are suboptimal for many
data types because they treat data as a byte string. Specialized
codecs that understand the data structure can be 2–3× better.

DwarFS has two we don't:

- **FLAC** for PCM audio (WAV, AIFF, raw) — 83% saved vs ~30% for
  general codecs.
- **Rice++** for FITS astronomical images — 68% saved vs ~40% for
  general codecs.

## Approach

Add two new codec ids and corresponding categorizers.

### FLAC (codec id 0x07 if FSST takes 0x07, else 0x08)

- Decoder: use `claxon` crate (pure Rust, MIT/Apache).
- Encoder: no pure-Rust FLAC encoder exists today. Options:
  - (a) Port from libFLAC (C, BSD-3-Clause). ~3 000 LOC. Acceptable
    license for LimniFS.
  - (b) Wait for omnizip to add FLAC (would need a new proposal).
- Categorizer: detect WAV (`RIFF....WAVE`) or AIFF (`FORM....AIFF`)
  magic, parse header to extract sample format (bits per sample,
  channels, sample rate, endianness). Pass to encoder.
- Wire format: representation triple codec byte = 0x07 (or 0x08).

### Rice++ (codec id 0x08 if FSST/FLAC take earlier slots)

- The ricepp library is small (~600 LOC, MIT license), already
  isolated from DwarFS at `src/external/dwarfs-t/` (via a vcpkg
  build). Could be:
  - (a) Wrapped via FFI (breaks pure-Rust guarantee).
  - (b) Ported to Rust. Small enough to port in a week.
- Categorizer: detect FITS magic (`SIMPLE  =`), parse header to
  extract `BITPIX` (bytes per sample), `NAXIS` (dimensions),
  endianness (always big-endian per FITS spec). Pass to encoder.

## Implementation sketch

```
file → file-level categorizer
        ├── FITS magic?     → ricepp codec with header params
        ├── WAV/AIFF magic? → FLAC codec with PCM params
        └── otherwise       → FastCDC + per-chunk classify (current)
```

Requires the file-level categorization refactor
(`04-file-level-categorization.md`, Tier 3 in the investigation).

## Acceptance criteria

- A 100 MB FITS file round-trips through LimniFS with ratio ≤ 35%
  (DwarFS achieves 32%).
- A 100 MB WAV file round-trips with ratio ≤ 18% (DwarFS achieves
  17%).
- New conformance vectors for FLAC and ricepp.
- No regression on existing benchmarks (PHP source, synthetic).

## CI evidence required

New benchmark dataset categories: `audio` (WAV corpus), `scientific`
(FITS corpus). Both must show ratio wins vs SquashFS / DwarFS
(parity with DwarFS expected; significant win vs SquashFS expected
since SquashFS uses general zstd).

## Out of scope

- Video codec specialization. Modern video (H.264/HEVC) is already
  compressed; STORE is correct. Uncompressed video is rare and
  benefits modestly from general codecs.
