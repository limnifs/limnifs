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

- [x] `find_boundary` SIMD path produces identical boundaries as
      scalar (determinism preserved). The 4× unroll does not change
      the algorithm — same `fp` recurrence, same mask checks, same
      boundaries. All 11 chunker tests pass unchanged.
- [~] Benchmark: large-file create speed improves ≥ 2×. **Partially
      met** — 10–15% measured improvement on chunking-heavy paths.
      The original 2× target assumed full SIMD, which the proposal
      at `docs/fastcdc-simd-proposal.md` shows requires an algorithm
      change (leap-based CDC) or nightly-only intrinsics (CLMUL).
      Shipped the realistic optimization; the bigger wins are filed
      as future work.

## Implementation notes (2026-08-05)

Shipped in v0.2.24. See `docs/fastcdc-simd-proposal.md` for the
full algorithmic analysis.

The TODO's original "process 16 bytes per iteration using
`wide::u64x2`" is not achievable: the gear hash has a true
loop-carried dependency (`fp[i+1]` depends on `fp[i]`), and
`wide` provides no gather-load that would let us look up 16 gear
values in parallel.

What shipped: 4× unrolled inner loops in `find_boundary`. The
four gear lookups per iteration can be hoisted into a vector load
by the optimiser; the four mask checks fold into a vector
compare. Sequential shift-and-add is unchanged (no SIMD win
there, but it was already 1 cycle per byte).

Full SIMD via either CLMUL (nightly `std::simd`) or leap-based
CDC (algorithmic change, breaks wire-format boundary compat) is
filed in `docs/fastcdc-simd-proposal.md` as future work.
