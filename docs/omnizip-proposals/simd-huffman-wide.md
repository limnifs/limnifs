# Proposal: SIMD Huffman decode — unblock with `wide` crate fallback

**Filed by:** LimniFS
**omnizip-rs crate:** `omnizip-zstd` (primary), `omnizip-deflate`, `omnizip-brotli`, `omnizip-bzip2`
**Severity:** unblock TODO 83

## Problem

`omnizip-rs TODO.complete/83-simd-huffman-decode.md` is **blocked**
on `std::simd::simd_gather` stabilising on stable Rust. The TODO
estimates a 1.5–3× throughput win on the Huffman inner loop, which
is the bottleneck for DEFLATE, Brotli, ZSTD, and BZip2 decode.

Today: every `cargo` user is on stable; `std::simd` is nightly-only.
The TODO is effectively dead in the water.

## Proposed unblock

Use the [`wide`](https://crates.io/crates/wide) crate, which provides
portable SIMD types (`u8x16`, `u32x8`, etc.) on stable Rust today.
`wide` doesn't expose a `gather` primitive directly, but the
Huffman inner loop doesn't need one — it needs **batched table
lookups**, which we synthesise from `wide`'s shuffle + bit manipulation.

### The technique

Standard table-driven Huffman decode is:

```text
loop:
    bits = peek(N)               # N = max code length
    sym = table[bits]            # one memory load
    consume(table[bits].len)     # update bit position
    output(sym)
```

The `consume` step creates a sequential dependency. The SIMD
version processes 8–16 symbols per iteration by:

1. Peeking 8 × N bits at once (8 separate bit positions,
   precomputed from the code-length distribution).
2. Performing 8 table lookups in parallel using a `u32x8` index
   vector — emulated via `wide`'s `u32x8::new(...)` + scalar
   fallback inside the wrapper. (The "gather" is a loop the
   compiler auto-vectorises; we don't need true SIMD gather.)
3. Writing 8 symbols to output via a single `u8x8` store.

The win is **not** from gather (which `wide` doesn't have); it's
from removing the sequential `consume` dependency by batching the
peek operations.

### Concrete interface

```rust
// In omnizip-zstd/src/huffman/simd.rs (new)
#[cfg(feature = "simd")]
pub fn decode_eight_symbols(
    table: &HuffmanTable,
    bits: [u32; 8],
    code_lens: [u8; 8],
) -> [u8; 8];

#[cfg(not(feature = "simd"))]
pub fn decode_eight_symbols(...) {
    // scalar fallback: loop 8 times
}
```

The `simd` feature is **opt-in** (off by default) to preserve the
sovereign build. LimniFS enables it for the `max-read` profile.

## Why `wide` instead of waiting for `std::simd`

| Path | Status | Portability | Adds dep? |
|---|---|---|---|
| `std::simd` | nightly only | x86 + ARM + WASM | no |
| `wide` | stable since 2021 | x86 SSE/AVX, ARM NEON | yes (small, no transitive) |
| `pulp` | stable, ARM + x86 | wider | yes (heavier) |

`wide` is the right choice today; we migrate to `std::simd` when
`simd_gather` stabilises (Rust 1.85+ forecast).

## Acceptance

- [ ] `decode_eight_symbols` exists behind a `simd` feature flag.
- [ ] On Enwik8 decompressed via ZSTD level 19, the SIMD path is
      ≥ 1.5× the scalar path's throughput.
- [ ] Output byte-identical to scalar (deterministic test).
- [ ] Default-feature build (no `simd`) is unchanged.

## Why LimniFS cares

LimniFS's `cat` command on a ZSTD-compressed image is bounded by
the ZSTD Huffman decoder. 1.5× faster decode = 1.5× faster `cat`.
For the `max-read` profile (write once, read many), this is the
single biggest read-path win available.

## Effort estimate

4 days:
- 1 day: scalar batching baseline (proves the batching win without SIMD).
- 2 days: `wide` SIMD batching in ZSTD Huffman.
- 1 day: differential tests + benchmarks.

After ZSTD lands, the same pattern applies to DEFLATE, Brotli,
BZip2 (each ~2 days more).

## Related

- omnizip-rs TODO 83.
- Kosolobov (2022), *Efficiency of ANS Entropy Encoders* — derives
  the batching bound theoretically.
- zlib-rs's SIMD Huffman (in C) — design reference.
