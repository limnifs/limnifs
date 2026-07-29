# 11 — mount

`limnifs-fuse`: mounting LimniFS images as filesystems. FUSE everywhere;
composefs-style kernel path on Linux when the cache exists.

- **Phase:** 1 (FUSE), 3 (composefs path)
- **Crate:** `limnifs-fuse`
- **Design refs:** §10.3 (kernel path), §12 (tebako cold-start requirement), §2 (no kernel driver in v1)

## Responsibilities (MECE)

**Owns:**

- FUSE daemon: inode table from 03's resolved tree, read-through to `DropSource`, mmap-friendly read path (cold start matters for tebako).
- Multi-image attach: several manifests mounted as overlay stacks (uses 03's resolver).
- composefs-style path: EROFS metadata loop mount + content-addressed cache filled via 08.

**Does NOT own:** image semantics (03), transport (08), caching policy beyond read-through, write-anything (images are immutable; mounts are read-only).

## Invariants

- Mounts are read-only, always. Mutation = build a delta (06), mount that.
- Bounded kernel-visible latency: metadata ops never trigger drop-store I/O.

## Performance budget

- tebako cold start: first byte of application code served ≤ 150 ms after mount on warm page cache, local image.

## Tasks

- [11-fuse-daemon.md](11-fuse-daemon.md)
- [11-composefs-path.md](11-composefs-path.md)
