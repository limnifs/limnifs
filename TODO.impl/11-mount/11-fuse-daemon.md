# 11 — FUSE daemon

- **Status:** done — limni/src/{vfs,fuse_vfs}.rs (behind fuse feature, fuser 0.18)
- **Phase:** 1
- **Depends on:** 03-overlay-resolver
- **Design refs:** §12 (tebako cold start), §2 (read-only mounts)

## Goal

Read-only FUSE mount: inode table from the resolved tree, read-through to
`DropSource`, multi-image overlay-stack mounts, mmap-friendly read path.

## Notes

- Metadata ops never trigger drop-store I/O (component invariant).
- Cold-start budget: first application byte ≤ 150 ms on warm page cache, local image (tebako requirement).
- `fuser`-class crate; macFUSE/FUSE-T and Linux both first-class (tebako platforms).

## Acceptance

- POSIX-ish behavior vectors (readdir/stat/read/seek patterns from the parity suite, 12) pass on Linux and macOS.
- Cold-start benchmark recorded against budget.
