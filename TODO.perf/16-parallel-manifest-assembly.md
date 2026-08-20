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

## Findings (2026-08-20, env-gated LIMNIFS_TRACE_ASSEMBLE instrumentation kept in assemble)

- [x] Slab hashing already parallel (verify) — pack_slabs rayon-parallels
      encode+hash per slab; 131 ms on a 50K-drop tree
- [x] B-tree build parallelized — N/A after the wire-format pivot: dir
      nodes are flat encoded nodes, no per-subtree build; encode is a
      memcpy loop (3.4 ms on 50K inodes)
- [x] Output bytes unchanged — no code path changed
- [ ] Benchmark: tiny-files improves measurably — REJECTED with data:
      the sequential steps are <1% of create (shared_inline_table
      137 ms + metadata_encode 207 ms on 50K inline files). The
      dominant cost is metadata_compress: 25.6 s single-stream Brotli
      q2 at ~2 MB/s on a 51 MB blob — codec-bound (omnizip's
      O(N x dict_words) dictionary lookup, BUGREPORT-brotli-dict-
      lookup-O(n).md upstream), not a parallelization gap. Per the
      no-profile-workarounds rule, the fix belongs upstream; local
      parallelism cannot split one Brotli stream without changing the
      wire format.
