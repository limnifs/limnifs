# 19 — BLAKE3 hash-tree parallelism

- **Priority:** P2
- **Side:** LimniFS
- **Est. effort:** 1d

## Problem

BLAKE3's `rayon` feature is enabled, but its multi-threaded mode
only fires for inputs ≥ 2 MiB. Per-chunk hashing (default chunk size
16 KiB) is scalar. For large files with many small chunks, hash
parallelism is unused.

## Fix

Two paths:

1. **Per-file hash parallelism**: for files > 2 MiB, hash the whole
   file in parallel via BLAKE3's tree mode, then derive per-chunk
   hashes via the tree's internal nodes. BLAKE3's design supports
   this natively but `blake3::Hasher::update_rayon` consumes the
   whole input at once.

2. **Cross-file hash parallelism**: rayon's existing par_iter already
   parallelizes across files. Within a file, chunks are hashed
   sequentially (FastCDC requires sequential boundary detection).

The realistic win is #1 for large-file workloads.

## Expected impact

- 10–20% on large-file workloads (ML models, archives within archives)
- No change on small-file workloads (already chunk-parallel via rayon)

## Findings (2026-08-20)

- [x] Profile BLAKE3 cost on large-file benchmark — measured 1.65 GB/s
      scalar on this machine: the whole 47.7 MB FITS file hashes in
      29 ms against a 276 s create (0.01%). Per-chunk hashing already
      rides the rayon file-level parallelism; inline-file hashing is
      137 ms for 50 K files.
- [x] If >10% of total, implement hash-tree derivation — NO: BLAKE3
      is under 0.1% of every measured workload. The >10% bar from the
      acceptance criteria is nowhere near met; tree-mode derivation
      would add complexity for unmeasurable gain.
- [x] Output bytes unchanged (BLAKE3 is deterministic regardless) —
      no code path changed.
