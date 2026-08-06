# 24 — `limni inspect` (image overview for debugging + SOTA demo)

- **Priority:** P1
- **Side:** LimniFS
- **Est. effort:** 4h

## Problem

There's no way to introspect a `.lim` image without writing custom
code: codec histogram, slab layout, drop size distribution, Merkle
root, manifest fields. For debugging, demos, and the SOTA story,
an `inspect` subcommand would surface all of this in one place.

## Fix

`limni inspect <image.lim>` prints:

```
Image: my-app.lim
Manifest root: blake3:7a3f...
Format version: 1
Base root: none (standalone image)

Slabs: 12 (3.4 GiB total, avg 284 MiB)
  Largest slab: 487 MiB (slab 7)
  Smallest slab: 12 MiB (slab 0)

Drops: 8,432 (3.4 GiB plaintext, 1.2 GiB compressed → 35.5% ratio)
Codec histogram:
  brotli   4,201 (49.8%)  → 920 MiB compressed (76.4%)
  zstd       891 (10.6%)  → 156 MiB compressed (13.0%)
  lz4        632 (7.5%)   → 89 MiB compressed (7.4%)
  store      708 (8.4%)   → 39 MiB compressed (3.2%)  [incompressible]
  ...

Inline: 1,932 inodes (2.4 MiB inline data, avg 1.3 KiB/inline)

Features: brotli-dict zstd-dict profile-descriptor
```

JSON output via `--json` for tooling.

## Expected impact

- Not a perf win — UX and SOTA demo
- Critical for adoption (people want to see what's inside)

## Acceptance

- [ ] `limni inspect` prints human-readable summary
- [ ] `--json` for machine-readable output
- [ ] Works on standalone images
- [ ] Works on layered images (shows base_root + chain depth)
