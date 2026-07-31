# 04 — Content-defined chunking (FastCDC)

- **Status:** done — limnifs-write/src/chunker.rs (FastCDC, Xia et al. 2016)
- **Phase:** 1
- **Depends on:** 03-drop-store-reader
- **Design refs:** §6 (ingest), §16 (open question 2)

## Goal

FastCDC chunker turning slices into drops (64 KiB–1 MiB target) with BLAKE3
`DropId` computation; parameter study recorded for min/avg/max vs. dedup
ratio and index size under slab packing.

## Notes

- Chunk parameters live in the manifest (per image), not hardcoded — re-chunking experiments don't fork the format.
- Streaming API: chunker consumes `impl Read`, constant memory.

## Acceptance

- Dedup benchmark vs. fixed-size chunking on tebako's real corpus (ruby runtimes) shows expected CDC win; numbers recorded in the task file.
- Boundary-shift stability: 1-byte insert shifts ≤ 2 chunk boundaries (vector).
