# 04 — Slab packing and drop GC

- **Status:** done — limnifs-write/src/{compaction,turnover}.rs
- **Phase:** 1
- **Depends on:** 04-ingest-epilimnion
- **Design refs:** §5 (slab packing, Taobao TFS lesson), §7 (turnover GC)

## Goal

Slab writer packing drops into 4–64 MiB objects with the two-level index; GC
of unreferenced drops during turnover (with 06).

## Notes

- Packing is locality-aware: drops of one slice land contiguously where possible (read-amplification budget depends on it).
- GC is mark-and-sweep from manifest roots; never runs without a live manifest lock.

## Acceptance

- Small-file vector (10k × 1–50 KiB files): reads touch ≤ 2 slab extents per file (Taobao TFS motivation, measured).
- GC vector: dropped deltas' unreferenced drops are reclaimed; referenced ones survive (bit-exact).
