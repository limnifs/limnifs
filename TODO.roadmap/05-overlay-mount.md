# 05 — Overlay mount (writable FUSE)

- **Priority:** P1
- **Depends on:** 02-epoch-format, fuse feature
- **Estimated effort:** 2 days

## Goal

Writable FUSE mount: overlay a writable directory on top of a read-only
`.lim` base image. Reads check overlay first; writes go to overlay.
`limni commit` converts overlay changes into an epoch.

## Design

```
limni mount-writable <image.lim> <overlay-dir> <mountpoint>
  → FUSE reads: check overlay-dir first, then base image
  → FUSE writes: write to overlay-dir
  → FUSE deletes: create whiteout marker in overlay-dir
limni commit <image.lim> <overlay-dir> [--output epoch-N.bin]
  → Diff overlay-dir vs base image tree
  → Produce epoch with Add/Remove/Replace operations
  → Epoch chains from base image's Merkle root
```

## Acceptance

- Mount succeeds on Linux + macOS (with fuse feature)
- Read from overlay takes priority over base
- Write creates files in overlay-dir
- `limni commit` produces a valid epoch
- Replaying the epoch reconstructs the overlay state
