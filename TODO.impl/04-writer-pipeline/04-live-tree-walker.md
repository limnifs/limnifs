# 04 — Live tree walker (DRY refactor)

- **Status:** DONE (parallel extract migrated 2026-08-04; compaction remains defensive flat-iteration by design)
- **Phase:** 2
- **Depends on:** 06-turnover, 03-drop-store-reader
- **Design refs:** 2026-throughput-roadmap.md §9
- **Priority:** ~~P1~~ closed

## Final resolution (2026-08-04)

`limnifs_core::live_tree` now provides:
- `LiveTreeSink` trait.
- `walk_live_tree` walker.
- `FilesystemSink` — single-threaded extract (RwImage).
- `ParallelExtractSink` — creates dirs inline, collects owned-Inode tasks for rayon.
- `DropIdCollectorSink` — for future compaction.
- `file_plaintext` — canonical extraction (sub-drop addressing bug fix).

All three tree-walking callsites now use the shared walker:
- `RwImage::write_live_dir` → `walk_live_tree` + `FilesystemSink`.
- `limni::extract` → `walk_live_tree` + `ParallelExtractSink`, rayon fan-out on the collected tasks.
- `limni::extract_file` → `file_plaintext`.

`extract_dir_collect` deleted from main.rs.

## Compaction (intentionally NOT migrated)

`limnifs_write::compaction::find_referenced_drops` flat-iterates all
inodes in the blob rather than walking from root. The semantic
difference is intentional: compaction defensively enumerates drops
even from unreachable inodes (corruption-tolerant). A `DropIdCollectorSink`
exists for future use but compaction's defensive iteration is a
feature, not duplication.

## Update (2026-08-04)

`limnifs_core::live_tree` now exists with:
- `LiveTreeSink` trait (`on_directory`, `on_regular_file`, `on_symlink`, `on_other`).
- `walk_live_tree(blob, root_inode, sink)` walker with cycle detection.
- `FilesystemSink` — writes the tree to a directory; used by `RwImage::write_live_tree`.
- `DropIdCollectorSink` — collects referenced DropIds (kept for future compaction migration).
- `file_plaintext(inode, slab_store)` — canonical file-extraction helper that honours `SliceRef::drop_byte_start` and `drop_byte_len` (the bug fix).

Migrated:
- `RwImage::write_live_dir` (limnifs-write/src/rw.rs) — replaced with `walk_live_tree` + `FilesystemSink`. ~75 lines removed.
- `limni::extract_file` (limni/src/main.rs) — replaced inline match with `limnifs_core::live_tree::file_plaintext`. Bug fix: now respects sub-drop slice addressing.

Two new tests in `live_tree::tests`.

## Remaining work

### Compaction (deferred)

`limnifs_write::compaction::find_referenced_drops` flat-iterates all
inodes in the blob rather than walking from root. The semantic
difference is intentional: compaction defensively enumerates drops
even from unreachable inodes (corruption-tolerant). A `DropIdCollectorSink`
is provided for future use; compaction's migration is deferred until
we want to consolidate the defensive iteration with the walker
(accepting that compaction no longer cleans up corrupt-image state).

### Parallel extract_dir_collect (deferred)

`limni::extract_dir_collect` walks the tree collecting
`(PathBuf, &Inode)` tasks that rayon then processes in parallel.
Migrating to `walk_live_tree` requires a sink variant that
collects owned `Inode` data (not borrows) so the parallel phase
can outlive the borrow. The current sink returns borrows; this
works for single-threaded `RwImage` but not for the parallel extract.

Plan when this lands: add a `FileTaskCollectorSink` that owns
clone of the inode data. The walker API stays unchanged.

## Goal

Three places walk the live tree of an opened image and materialise
files to disk:

- `limni/src/main.rs::extract_dir_collect` + `extract_file`
- `limnifs-write/src/rw.rs::RwImage::write_live_dir`
- (effectively) `limnifs-write/src/compaction.rs::find_referenced_drops`

Each re-implements directory traversal, hash lookup, inline-data
extraction, and slice-map reconstruction. They have drifted: the
CLI's slice-map extraction ignores `drop_byte_start` and
`drop_byte_len`, which works only because the writer never emits a
slice that doesn't span the whole drop. The `RwImage` copy has the
same bug. The compaction version skips extraction entirely.

Extract one walker, parameterised by a `LiveTreeSink` trait, that
all three call sites use.

## Acceptance

- [x] `LiveTreeSink` trait + `walk_live_tree` exist in `limnifs-core::live_tree`.
- [x] `FilesystemSink` writes the live tree to disk.
- [x] `DropIdCollectorSink` collects referenced DropIds.
- [x] `file_plaintext(inode, slab_store)` canonical helper honours sub-drop addressing.
- [x] `RwImage::write_live_dir` deleted; replaced with `walk_live_tree`.
- [x] `limni::extract_file` migrated; sub-drop addressing bug fixed.
- [ ] `limni::extract_dir_collect` migrated to a parallel-friendly sink variant.
- [ ] `compaction::find_referenced_drops` migrated (or documented as intentionally defensive).
- [x] All existing tests still pass.
- [x] New tests cover the walker + sink.
