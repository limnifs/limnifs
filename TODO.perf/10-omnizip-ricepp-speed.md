# 10 — omnizip: ricepp encoder speed

- **Priority:** P0
- **Side:** omnizip (file as proposal)
- **Est. effort:** 3d (omnizip-side)

## Problem

Benchmark `fits-synthetic` (48 MB FITS): LimniFS 23.9s create vs
DwarFS 3.9s. LimniFS achieves **32.08% ratio** (31% better than
DwarFS's 46.29%), but the encoder is **6× slower**.

omnizip-ricepp does per-pixel Rice encoding sequentially. DwarFS's
ricepp (C++) uses SIMD byte-shuffle before Rice encoding.

## Proposal

File to omnizip-rs:
1. SIMD byte-shuffle: transpose pixel bytes (e.g. f32 → 4 lanes of
   exponents, mantissas, signs) before Rice encoding. Exposes
   redundancy the Rice encoder can exploit with fewer bits.
2. Parallel block processing: process FITS blocks in parallel via
   rayon (each block is independent).
3. Lookup-table Rice encoding: precompute the Rice codeword for
   every possible 16-bit residual (65536 entries × 4 bytes = 256 KB
   LUT). Eliminates the bit-by-bit encoding loop.

## Expected impact

- LimniFS FITS create: 23.9s → ~4s (within 2× of DwarFS).
- Ratio may improve slightly (byte-shuffle exposes more redundancy).

## Acceptance

- [ ] omnizip-ricepp encode speed improves ≥ 4×.
- [ ] Output ratio same or better.
- [ ] LimniFS FITS benchmark shows speed improvement.
