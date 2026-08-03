# 04 — Live tree walker (DRY refactor)

- **Status:** pending
- **Phase:** 2
- **Depends on:** 06-turnover, 03-drop-store-reader
- **Design refs:** 2026-throughput-roadmap.md §9
- **Priority:** P1

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

## Design

```rust
pub trait LiveTreeSink {
    fn on_directory(&mut self, abs_path: &Path) -> Result<()>;
    fn on_regular_file(&mut self, abs_path: &Path, plaintext: &[u8]) -> Result<()>;
    fn on_symlink(&mut self, abs_path: &Path, target: &str) -> Result<()>;
    fn on_other(&mut self, abs_path: &Path, inode: &Inode) -> Result<()>;
}

pub fn walk_live_tree(
    blob: &MetadataBlob,
    root_inode_number: u64,
    slab_store: Option<&SlabStore>,
    sink: &mut dyn LiveTreeSink,
) -> Result<()>;
```

Three sinks:
- `FilesystemSink` (writes to a directory tree; used by extract,
  RwImage commit/turnover).
- `DropIdCollector` (records referenced DropIds; used by
  compaction).
- `ByteIdentitySink` (used by turnover's byte-identity test).

## Notes

- The slice-map sub-drop addressing bug becomes a single fix
  instead of three.
- Symlinks/devices/pipes get a real code path for the first time
  (today `RwImage` silently drops them).

## Acceptance

- [ ] `walk_live_tree` lives in `limnifs-core::live_tree` and is
      exported.
- [ ] `extract`, `RwImage::commit`, `RwImage::turnover`, and
      `compaction::compact_image` all call it via their respective
      sinks.
- [ ] Sub-drop slice addressing (`drop_byte_start`/`drop_byte_len`)
      is honoured in the FilesystemSink.
- [ ] A new test exercises a multi-slice drop and asserts the
      reconstructed plaintext is correct.
- [ ] All existing tests still pass.
