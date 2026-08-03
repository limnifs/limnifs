# 04 — Cross-image sparse index (dedup across images)

- **Status:** pending
- **Phase:** 2
- **Depends on:** 03-drop-store-reader, 04-deepening-compactor
- **Design refs:** §6, 2026-throughput-roadmap.md §2, Lillibridge FAST'2009
- **Priority:** P1

## Goal

Per-image dedup is done (a `DropId` is BLAKE3 of plaintext; the
writer's `drop_index` HashMap deduplicates within one image). For
multi-image workloads (container layering, versioned datasets, CI
artifact archives), the same drop is recompressed for every image
that contains it. A cross-image sparse index lets the writer ask
"is this drop already in image X?" before doing work.

## Design

1. A sidecar file `image.lim.sparse` containing a Bloom filter of
   the image's `DropId` set (truncated to first 16 bytes; FPP
   configurable).
2. `SparseIndexWriter::insert(DropId)`, `SparseIndexReader::probably_contains(DropId)`.
3. The writer walks a list of `SparseIndexReader`s before
   recompressing; on a hit, it copies the slab record (not the
   plaintext) into the new image's slab and rewrites the locator.
4. False positives cost one extra slab read; acceptable.

## Notes

- The Bloom filter is small (≈ 10 bits per drop at FPP 1%). A 1M
  drop image's index is ~1.2 MB.
- This TODO is blocked on a real multi-image workload appearing in
  `limnifs-bench`. Today's benchmarks are single-image, so there's
  no measurable win yet.

## Acceptance

- [ ] Spec exists.
- [ ] `SparseIndexReader/Writer` types exist with tests; not yet
      wired into the writer.
- [ ] Wired into the writer once a benchmark exercises a real
      multi-image workload.
