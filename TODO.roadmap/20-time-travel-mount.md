# time travel mount

- **Priority:** P2
- **Depends on:** 05,15
- **Estimated effort:** see detail

## Goal

Time-travel FUSE mount.

## Detail

limni mount --epoch K <image> <mountpoint>. Mounts the filesystem at a specific epoch. Uses persistent tree (task 15) for O(1) state reconstruction.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
