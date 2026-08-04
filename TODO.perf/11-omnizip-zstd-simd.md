# 11 — omnizip: ZSTD SIMD encode

- **Priority:** P1
- **Side:** omnizip (file as proposal)
- **Est. effort:** 5d (omnizip-side)

## Problem

omnizip-zstd's encoder is pure Rust. On source code, it achieves
~400 MB/s at L6. The C reference (libzstd) achieves ~800 MB/s at
L6 on the same hardware. The gap is primarily in the match finder
(hash chain walk) and entropy coder (FSE table construction).

## Proposal

File to omnizip-rs:
1. SIMD hash computation: compute the 4-byte rolling hash using
   `wide::u32x4` (4 positions per iteration).
2. Vectorised match compare: use `wide::u8x16` to compare candidate
   match positions 16 bytes at a time.
3. Lookup-table FSE state transitions: precompute the state machine
   transition table for common accuracy_log values (5, 6). Avoids
   the per-symbol division in the decode path.

## Expected impact

- ZSTD encode throughput: +50–100%.
- LimniFS `balanced()` create speed: proportional improvement on
  text-heavy workloads.

## Acceptance

- [ ] omnizip-zstd encode speed improves ≥ 1.5×.
- [ ] Output ratio unchanged.
