# 07 — Reed-Solomon slab coding

- **Status:** done — limnifs-core/src/{gf256,reed_solomon}.rs (systematic Vandermonde)
- **Phase:** 3
- **Depends on:** 04-slab-packing-gc
- **Design refs:** §8 (EC), §4 (identity-neutral representation)

## Goal

`Ec` trait + Reed-Solomon (k+m) per-slab encode/decode; shard layout recorded
per manifest schema; reconstruction verified against slab hash before yield.

## Notes

- Mixed-EC images legal (per-slab k,m); OCP leaves room for fountain codes later.
- Throughput target: encode ≥ 500 MB/s on modern x86 (SIMD backend, e.g. `reed-solomon-simd`-class crate).

## Acceptance

- Loss vectors: slab reconstructible from any k of k+m shards; hash verification on reconstruction; identity unchanged (same `DropId`s).
