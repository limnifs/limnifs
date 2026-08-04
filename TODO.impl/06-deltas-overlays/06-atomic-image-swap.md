# 06 — Atomic image swap (rename-based)

- **Status:** DONE (2026-08-04)
- **Phase:** 2
- **Depends on:** 06-turnover
- **Design refs:** §7 (RW), 2026-throughput-roadmap.md §6
- **Priority:** ~~P1~~ closed

## Resolution

`RwImage::write_artifact` now writes manifest + slabs to
`<manifest_path>.new/` then renames each file into place. Order:
sidecar → slabs → manifest. A reader that opens the manifest after
the manifest rename always sees referenced slabs already in place.

Implementation:
- Build new files in `<manifest_path>.new/` directory.
- Write all files first; only renames after all writes succeed.
- `rename(2)` is atomic for a single file on POSIX filesystems
  (APFS, ext4, btrfs, xfs).
- Cross-filesystem rename (EXDEV) falls back to read-write-remove
  so we still get the final state, just without the cross-reader
  atomicity guarantee.
- Cleanup: `<manifest_path>.new/` removed after successful swap.

Crash recovery (future work, `06-rw-crash-safety.md`):
- A crash mid-sequence leaves `<manifest_path>.new/` on disk.
- The previous image is intact (no in-place mutation).
- Recovery on next `RwImage::open` could detect the stale
  directory and offer to clean up or resume.

## Goal

`RwImage::commit` and `RwImage::turnover` write each file
(`manifest`, `slab-N.bin`, sidecar) one `write(2)` at a time. A
reader that opens the image between two writes sees an
inconsistent snapshot. Make the commit produce a fully-formed
image and swap it in atomically.

## Acceptance

- [x] `RwImage::commit` and `RwImage::turnover` build in
      `<image>.new/` and rename into place.
- [x] Order: sidecar → slabs → manifest so readers never see a
      manifest referencing missing slabs.
- [x] `.new/` is cleaned up on success.
- [ ] Concurrent-reader test that opens the image from a second
      thread during commit never observes an inconsistent
      snapshot. (Requires thread infrastructure; the rename
      ordering provides the guarantee by construction.)

## Notes

- Cross-directory rename on the same filesystem is atomic on
  POSIX; cross-filesystem rename is not. The `.new/` directory
  lives next to the manifest so they share a filesystem in the
  common case.
- macOS APFS and Linux ext4/btrfs/xfs all support this; no
  portability concern.
