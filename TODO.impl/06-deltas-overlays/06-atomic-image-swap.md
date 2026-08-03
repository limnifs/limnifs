# 06 — Atomic image swap (rename-based)

- **Status:** pending
- **Phase:** 2
- **Depends on:** 06-turnover
- **Design refs:** §7 (RW), 2026-throughput-roadmap.md §6
- **Priority:** P1

## Goal

`RwImage::commit` and `RwImage::turnover` write each file
(`manifest`, `slab-N.bin`, sidecar) one `write(2)` at a time. A
reader that opens the image between two writes sees an
inconsistent snapshot. Make the commit produce a fully-formed
image and swap it in atomically.

## Design

- Build the new image in `<image>.new/`.
- `rename(2)` each file from `<image>.new/<name>` to the live
  location. `rename(2)` is atomic for a single file; for the
  multi-file image we order the renames so the manifest lands
  last.
- Order: sidecar → slabs → manifest. A reader that opens the
  manifest sees a consistent snapshot only after the manifest
  rename completes.

## Notes

- This is the foundation for `06-rw-crash-safety.md`.
- Cross-directory rename on the same filesystem is atomic on
  POSIX; cross-filesystem rename is not. The `.new/` directory
  must live on the same filesystem as the image.
- macOS APFS and Linux ext4/btrfs/xfs all support this; no
  portability concern.

## Acceptance

- [ ] `RwImage::commit` and `RwImage::turnover` build in
      `<image>.new/` and rename into place.
- [ ] A test opens the image from a second thread while commit is
      running and never observes an inconsistent snapshot.
- [ ] `.new/` is cleaned up on success.
