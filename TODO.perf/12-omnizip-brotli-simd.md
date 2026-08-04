# 12 — omnizip: Brotli SIMD encode

- **Priority:** P2
- **Side:** omnizip (file as proposal)
- **Est. effort:** 5d (omnizip-side)

## Problem

The `brotli` crate (by Daniel Reiter Horn) is pure Rust at ~200 MB/s
(q5). The C libbrotli at q5 achieves ~400 MB/s. The gap is in the
backward-reference finder (hash chain) and the Huffman literal
coder.

## Proposal

Same SIMD techniques as ZSTD (SIMD hash, vectorised match compare).
Lower priority because Brotli q5 at 200 MB/s is already "good enough"
for LimniFS's balanced profile — the bottleneck has moved to FastCDC
chunking + tournament overhead.

## Acceptance

- [ ] omnizip-brotli encode speed improves ≥ 1.5× at q5.
- [ ] Output ratio unchanged.
