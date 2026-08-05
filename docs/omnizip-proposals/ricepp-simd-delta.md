# omnizip-ricepp: SIMD delta encode (TODO 113)

- **omnizip version affected:** 0.14.12
- **LimniFS version:** 0.2.27
- **Filed:** 2026-08-05
- **Status:** half-of-gap closed upstream; remaining work filed

## Summary

`omnizip-ricepp` 0.14.12 ships TODO 113 — `simd-delta` feature with
`wide::u64x4`. Per-pixel zigzag delta encoding is now vectorised.
Half of the previously-measured 6× gap is closed.

## LimniFS impact

ricepp is the chosen codec for FITS images via the
`process_whole_file_drop` file-level categorizer path. Benchmark on
the synthetic `fits-synthetic` dataset (47 MB) with the
`balanced` profile:

| omnizip version | Create time | Notes |
|---|---|---|
| 0.14.11 | ~25.2 s | scalar delta |
| **0.14.12** | **~23.6 s** | **wide::u64x4 delta (~6% faster)** |

The remaining ~3× gap vs raw ZSTD on FITS is mostly the encoding
decision itself (ricepp is a specialised integer-pixel codec and
ZSTD has a richer general-purpose model). LimniFS does not currently
need to do anything to receive the speedup; the upgrade is the
bump.

## Remaining work

Filed at omnizip TODO 113 (the other half). Likely candidates:

- Vectorize the unary-bit emission (currently scalar per-bit
  loop). Dominant in the encode loop once delta is SIMD.
- Batch the per-pixel Huffman/Rice code selection across blocks.

LimniFS does not own the ricepp encoder; these are upstream items.
When they land, this proposal can be marked done.
