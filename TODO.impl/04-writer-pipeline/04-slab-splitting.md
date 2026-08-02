---
Component: 04-writer-pipeline
Task: 04-slab-splitting
Status: done (2026-08-01, session 31)
Depends on: 04-slab-packing-gc
Unblocks: —
Source of bug: limnifs-bench, session 30 (2026-08-01)
Fix landed: limnifs-write/src/lib.rs pack_slabs + encode_slab(ordinal);
  limnifs-core/src/slab_store.rs SlabStore for reader-side multi-slab
  lookup. Test: slab_splits_when_content_exceeds_ceiling.
---

# 04-slab-splitting — Writer must split slabs that exceed the 64 MiB reader ceiling

## Problem

`limnifs-write::write_directory` packs every drop into a single slab and
returns it via `WriteArtifact::slab_bytes`. The reader's slab parser
(`limnifs-core::slab::parse_slab_header`) rejects any slab whose
`total_length > DEFAULT_SLAB_MAX_BYTES` (= 64 MiB) with
`CoreError::Corrupt`.

Result: any image whose compressed slab exceeds 64 MiB is unreadable.
`limni extract`, `limni cat`, and `limni mount` all fail.

## Reproducer (from session 30 benchmark)

```text
$ limni extract .scratch/bench-work/random/limnifs.lim /tmp/x
limni: ...limnifs.lim: manifest corrupt: slab total_length 104877048
exceeds configured ceiling 67108864
```

Dataset: `random` (100 MB random bytes). Compressed slab is ~100 MB
because random data is incompressible. Same failure on the `php` source
dataset (140 MB) once the metadata-blob ceiling (see
`04-metadata-externalization.md`) is also raised.

## Why this is a v0.2 P0

The v0.1 reader is correct to enforce a ceiling — unbounded
pre-allocation is a DoS vector. The bug is on the **writer** side: the
writer must split slabs before they exceed the reader ceiling. A
filesystem that cannot round-trip a 100 MB file is not a v1.

## Approach (per spec §3.1)

The slab index already supports multiple slabs (`slab_index: Vec<SlabIndexEntry>`
in the manifest). The writer currently always emits exactly one slab.
The fix is in `limnifs-write::lib::Writer::assemble`:

1. Track accumulated slab bytes after each drop is appended.
2. When appending the next drop would exceed
   `DEFAULT_SLAB_MAX_BYTES - SLAB_HEADER_LEN - SAFETY_MARGIN`, flush the
   current slab (record its `SlabId`, update `slab_index`), then start a
   new slab.
3. Each drop record already carries `slab_ordinal` via its `SlabId`; the
   metadata→slab→drop walk in the reader already supports multi-slab
   resolution.

The OCP win: no reader change. The reader already supports N slabs; the
writer just needs to produce N slabs instead of 1.

## Acceptance criteria

- `limni extract` round-trips the `random` (100 MB) and `php` (140 MB)
  benchmark datasets without error.
- New unit test in `limnifs-write` that synthesises >64 MiB of
  compressed-equivalent content and asserts the writer produces ≥2 slabs,
  each ≤64 MiB.
- Conformance vector added: image with multi-slab `slab_index`, both
  Rust and Python readers parse it.
- `limnifs-bench run --datasets random,php` shows non-empty extract
  rows for the limnifs column.

## CI evidence required

`phase-1-exit.yml` must stay green; `benchmark.yml` (full mode) must
show LimniFS extract times for the random and php datasets.
