# 04 — Deepening compactor

- **Status:** done — limnifs-write/src/lib.rs deepen_drop (LZ4 per class)
- **Phase:** 1
- **Depends on:** 04-ingest-epilimnion
- **Design refs:** §6 (metalimnion → hypolimnion), §16 (open question 1: solid blocks)

## Goal

Background re-encoding: zstd default, lzma/brotli for cold text classes, store
for incompressible; emits new `Representation`s without touching `DropId`s or
references. Includes the solid-block boundary decision (spec-driven).

## Notes

- Referential transparency is the invariant: deepening must be invisible to readers except ratio/speed.
- Runs under a configurable I/O budget; cancel-safe with valid intermediate manifests.
- Policy triggers: idle, age, size threshold, explicit `limni deepen`.

## Acceptance

- Ratio on tebako corpus meets or beats equivalent dwarfs-t image built with comparable settings; numbers recorded.
- Identity vector: pre/post-deepening images resolve byte-identical trees and identical `DropId` sets.
