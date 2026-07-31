# 04 — Epilimnion ingest

- **Status:** done — limnifs-write/src/lib.rs INLINE_THRESHOLD=4096; drop dedup via BLAKE3
- **Phase:** 1
- **Depends on:** 04-classifier-seine
- **Design refs:** §6 (quick write), §12 (CI artifact use case)

## Goal

The write path: chunk → classify → LZ4/store → pack into slabs → emit
manifest. Write latency ≈ memcpy + hash; no deepening on the ingest path.

## Notes

- `Sink` trait targets: local dir now; locator push (08) later — same code path (encapsulation).
- Cancel-safe: interrupted ingest leaves a valid incomplete staging area, resumable.
- Dedup at ingest: existing `DropId`s are referenced, not rewritten.

## Acceptance

- Ingest throughput ≥ 80% of memcpy+BLAKE3 baseline (benchmark recorded).
- Ingest → read round-trip passes conformance vectors end to end.
