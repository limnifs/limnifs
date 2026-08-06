# 16 — Parallel manifest assembly

- **Priority:** P1
- **Side:** LimniFS
- **Est. effort:** 4h

## Problem

`WriteContext::assemble` at end of write is sequential:
1. Build directory B-tree from inodes
2. Encode metadata blob
3. Compress metadata blob
4. Compute slab content hashes
5. Build manifest

For images with 100K+ inodes (container images, source trees), the
B-tree build and metadata encode take measurable wall-clock time on
a single core.

## Fix

Parallelize the steps that are CPU-bound and have parallelism:
- Directory B-tree build: rayon over sub-trees (per top-level entry)
- Slab content hashing: rayon over slabs (`pack_slabs` already does this)
- Drop-record encoding: rayon over per-slab drop lists

Steps that must stay sequential: manifest construction (single
writer), final manifest hash, artifact assembly.

## Expected impact

- ~20% on tiny-files (50K inodes)
- ~10% on container images (100K inodes)
- Negligible on small images

## Acceptance

- [ ] B-tree build parallelized
- [ ] Slab hashing already parallel (verify)
- [ ] Output bytes unchanged
- [ ] Benchmark: tiny-files improves measurably
