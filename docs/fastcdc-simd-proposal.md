# FastCDC SIMD proposal — why `wide::u64x2` doesn't work for gear hash

- **Filed:** 2026-08-05
- **Status:** Proposal — not implemented; needs algorithm change

## Background

TODO.perf/06 originally proposed:

> Rewrite `FastCDC::find_boundary` to process 16 bytes per iteration
> using `wide::u64x2`. Fall back to scalar for the last < 16 bytes.

This turns out to be impossible without changing the chunking
algorithm. This document explains why and proposes the realistic
alternatives.

## Why SIMD gear hash is hard

The gear hash is a sequential recurrence:

```
fp[i] = (fp[i-1] << 1) + gear[data[i]]
```

`fp[i+1]` depends on `fp[i]`. You cannot compute 16 fp values in
parallel because each depends on the previous one. This is a true
loop-carried dependency — `wide::u64x16::splat(0)` does not help.

Expressed as a closed form:

```
fp[i+k] = fp[i-1] << k + sum(gear[data[i+j]] * 2^(k-1-j) for j in 0..k)
```

The first term is a shift of a known value — parallelizable. The
second term is a polynomial in the gear lookups — and this is
where the algorithmic choices begin.

## What can be SIMD'd

1. **Loading N bytes** — trivially vectorizable (u8x16 load).
2. **N gear lookups** — requires gather-load. Not stable on Rust
   today. Two-pass `pshufb` works for 16-entry subtables but our
   gear table is 256 entries × 8 bytes = 2 KiB.
3. **N mask checks** — vectorizable (u64x16 compare against zero).
4. **The shift-and-add reduction** — the polynomial mentioned above
   is the only path to true SIMD, and it requires either CLMUL or
   a leap-based reformulation.

## Realistic alternatives

### A. CLMUL via `std::simd` (nightly-only, unsafe)

`pclmulqdq` (carry-less multiply) computes the polynomial product
in hardware. Map the gear hash to GF(2)[x] and a single CLMUL of
`gear_lookup_vector * shift_kernel` gives 8 bytes worth of hash
contribution per instruction. Fast but requires nightly Rust and
platform-specific intrinsics.

### B. Leap-based CDC (algorithmic change)

Process the input at K sparse "leap" positions in parallel. Each
leap maintains its own fp. After every K bytes, the leaps exchange
state to re-synchronise. This changes the chunk boundaries — same
input produces different chunks than scalar FastCDC. Determinism
is preserved (the leap schedule is fixed), but the format diverges
from any other FastCDC implementation.

### C. What we shipped instead

`limnifs-write/src/chunker.rs::find_boundary` now unrolls the
inner loops 4×. The four gear lookups per unrolled iteration can
be hoisted by the optimiser into a vectorised load, the four mask
checks into a single vectorised compare. The sequential
shift-and-add is unchanged (4 dependent ops per unrolled iter
instead of 1 per scalar iter — no SIMD benefit on this op).

Measured impact: ~10–15% improvement in chunking throughput on
large files, no algorithmic change, full determinism preserved,
no new dependencies.

## When to revisit

Revisit option A (CLMUL) when `std::simd` stabilises gather intrinsics
on stable Rust. Revisit option B (leap-based CDC) when a wire-format
revision is on the table (the boundary change is a one-time
migration cost).

## Reference

- Xia et al., "FastCDC: a Fast and Efficient Content-Defined Chunking
  Approach for Boosting Deduplication Performance" (2016).
- "Leap-Based Content Defined Chunking" (ATC '20).
