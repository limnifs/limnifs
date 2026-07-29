# 11 — Composefs-style kernel path

- **Status:** pending
- **Phase:** 3
- **Depends on:** 11-fuse-daemon, 08-locator-trait-registry
- **Design refs:** §10.3

## Goal

Linux fast path: EROFS metadata loop mount with drops served from the
content-addressed local cache (filled via 08), composefs-style.

## Notes

- Optional path; FUSE remains the portable default (design §2).
- Cache eviction policy documented; integrity re-verified on cache fill (lying-cache vector).

## Acceptance

- Same parity vectors as FUSE pass on the kernel path; cold-start beats FUSE baseline on repeated mounts (numbers recorded).
