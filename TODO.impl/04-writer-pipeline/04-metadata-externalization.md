---
Component: 04-writer-pipeline
Task: 04-metadata-externalization
Status: done (2026-08-01, session 31)
Depends on: 03-manifest-parser
Unblocks: —
Source of bug: limnifs-bench, session 30 (2026-08-01)
Fix landed: limnifs-write::lib::assemble emits a MetadataSidecar when
  the blob exceeds METADATA_EXTERNALIZE_THRESHOLD (768 KiB); limni
  CLI's load_image follows file: locators when the blob is external.
---

# 04-metadata-externalization — Writer must externalize metadata blobs > 1 MiB

## Problem

`limnifs-write::write_directory` inlines the entire metadata blob into
the manifest's `metadata_reference` section. The reader's
`metadata_reference` parser
(`limnifs-core::metadata_reference::parse_metadata_reference`) rejects
any inline blob > `DEFAULT_LOCATOR_MAX_INLINE_METADATA_BYTES` (= 1 MiB)
with `CoreError::Corrupt`.

Result: any image with many inodes (so the metadata blob exceeds 1 MiB)
is unreadable. `limni extract`, `limni cat`, `limni mount` all fail.

## Reproducer (from session 30 benchmark)

```text
$ limni extract .scratch/bench-work/php/limnifs.lim /tmp/x
limni: ...limnifs.lim: manifest corrupt: metadata_reference
inline_metadata_len 20481859 exceeds ceiling 1048576
```

Datasets affected (inode counts):
- `php` source: ~22 600 inodes → 19.5 MiB metadata blob
- `tiny-files` (50 000 × 17-byte files): 50 000 inodes → 4.0 MiB blob
- Any tree over ~1 200 inodes (depends on path lengths) hits the ceiling

## Why this is a v0.2 P0

The reader ceiling exists for the same DoS-protection reason as the slab
ceiling. The writer must externalize the blob (write it as a sidecar
file and reference it via `metadata_reference.locators`) instead of
inlining it. This is the spec-intended design — `metadata_reference`
supports both modes; the writer only uses inline today.

## Approach (per spec §5.3)

`MetadataReference` already has two modes:

```text
metadata_reference {
  metadata_hash: Hash,            // BLAKE3(blob)
  locator_count: u32,             // external blob locators
  locators: [LocatorEntry; locator_count],
  inline_metadata_len: u32,       // 0 when externalized
  inline_metadata: [u8; inline_metadata_len]
}
```

Fix in `limnifs-write::lib::Writer::assemble`:

1. After building the metadata blob, check its byte length.
2. If `blob.len() > DEFAULT_LOCATOR_MAX_INLINE_METADATA_BYTES / 2`
   (half the reader ceiling, leaving headroom), emit it as a sidecar
   `.metadata` file next to the slab, push a `file:` locator into
   `metadata_reference.locators`, set `inline_metadata_len = 0`.
3. Otherwise inline as today.

The OCP win: no reader change. The reader already handles both modes
correctly. The writer just needs to pick the right one based on size.

## Acceptance criteria

- `limni extract` round-trips the `php` source dataset (22 600 inodes)
  without error.
- `limni extract` round-trips a synthetic 50 000-file tree without
  error.
- New unit test in `limnifs-write` that synthesises >2 MiB of inodes
  and asserts the writer emits a non-empty `locators` list and
  `inline_metadata_len == 0`.
- Conformance vector added: image with externalized metadata, both
  Rust and Python readers parse it.
- `limnifs-bench run --datasets php,tiny-files` shows non-empty extract
  rows for the limnifs column.

## CI evidence required

`phase-1-exit.yml` must stay green; `benchmark.yml` (full mode) must
show LimniFS extract times for the php and tiny-files datasets.

## Out of scope (do NOT do here)

- Tuning the 1 MiB ceiling. The ceiling is correct; the writer must
  respect it.
- Compression of the metadata blob. That's a v0.3 optimisation.
