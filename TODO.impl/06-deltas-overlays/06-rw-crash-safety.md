# 06 — RW crash safety (write-ahead log)

- **Status:** pending
- **Phase:** 2
- **Depends on:** 06-turnover
- **Design refs:** §7 (RW), 2026-throughput-roadmap.md §6
- **Priority:** P1

## Goal

`RwImage::commit` writes the manifest, then each slab, then the
sidecar in sequence. A crash mid-sequence leaves the image in an
inconsistent state (manifest references drops the slab doesn't
have, or vice versa). Add a write-ahead log so recovery on `open`
can detect and roll back a partial commit.

## Design

1. `<image>.wal` records every pending operation since the last
   successful commit.
2. `RwImage::commit`:
   a. Write WAL with the planned operations.
   b. Write manifest + slabs + sidecar to `<image>.new/`.
   c. `rename(2)` `<image>.new/manifest` over `<image>`.
   d. `rename(2)` `<image>.new/slabs/*` over the slab files.
   e. Truncate WAL.
3. `RwImage::open` checks for a WAL; if non-empty, replay from the
   last committed image (i.e., skip the partial `<image>.new/`).

## Notes

- The WAL is intentionally simple: each entry is `op_kind |
  path_len | path | size | bytes`, length-prefixed and appended.
  No partial-entry recovery — if the WAL is truncated mid-write,
  the last entry is discarded.
- This doc depends on `06-atomic-image-swap.md`; the rename-based
  atomic swap is the actual crash-safety primitive, the WAL is the
  user-facing "what about my pending changes" story.

## Acceptance

- [ ] Spec exists.
- [ ] WAL append/truncate/replay round-trips in unit tests.
- [ ] A simulated crash (kill -9 between step b and d) leaves a
      recoverable image.
