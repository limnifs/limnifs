# 06 — FastCDC SIMD gear hash

- **Priority:** P1
- **Side:** LimniFS
- **Est. effort:** 3d

## Problem

FastCDC's inner loop is `fp = (fp << 1) + gear[byte]`. This is a
scalar per-byte operation. On 10 cores, we process ~700 MB/s for
repetitive data — limited by the single-threaded chunking pass
(parallel compress happens AFTER chunking).

SIMD gear hash: process 16 bytes per iteration using `u64x2`
(available via `wide` crate on stable Rust). The mask check
(`fp & mask == 0`) is also vectorizable.

## Fix

1. Add `wide = "0.7"` as a dep (portable SIMD, stable Rust).
2. Rewrite `FastCDC::find_boundary` to process 16 bytes per
   iteration using `wide::u64x2`.
3. Fall back to scalar for the last < 16 bytes.

## Expected impact

- **Chunking throughput**: 2–4× improvement on large files.
- **Create speed**: proportional improvement on single-file
  workloads (chunking is the bottleneck).

## Acceptance

- [ ] `find_boundary` SIMD path produces identical boundaries as
      scalar (determinism preserved).
- [ ] Benchmark: large-file create speed improves ≥ 2×.
