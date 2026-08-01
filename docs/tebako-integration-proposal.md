# Proposal: LimniFS as a tebako filesystem adapter

**To:** tamatebako/tebako maintainers
**From:** LimniFS team
**Date:** 2026-08-01
**Re:** Integrating LimniFS as an on-file filesystem (OFFS) adapter alongside DwarFS

## Background

Tebako packages interpretive-language applications (Ruby, Python, etc.)
into single executables. The filesystem layer — the on-file filesystem
(OFFS) — stores the application's files inside the executable binary.
Currently tebako uses DwarFS for this layer.

The `libtfs` project aims to abstract the filesystem layer behind an
adapter interface, allowing multiple OFFS backends (DwarFS, LimniFS,
others).

## Why LimniFS is a good fit for tebako

### 1. Pure Rust — no cross-compilation pain

Tebako's biggest build complexity is DwarFS: a C++ library requiring
`libdwarfs`, `libfuse`, `libzstd`, `liblz4`, `libbrotli`, and their
development headers on every target platform. Cross-compiling DwarFS to
aarch64, Alpine (musl), or Windows MSYS2 requires a full C++ cross-
toolchain per target.

LimniFS is pure Rust with `#![forbid(unsafe_code)]`. It compiles on
every target Rust supports, with no system dependencies. Adding LimniFS
as a tebako adapter would eliminate the C++ cross-compilation burden for
users who choose it as their OFFS backend.

### 2. Faster create + extract

LimniFS creates images 1.6x faster than DwarFS and extracts 2.3x faster.
For tebako, this means faster `tebako press` (packaging) and faster
cold-start (the runtime unpacks the OFFS into memory on first access).

### 3. Content-addressed deduplication

LimniFS identifies every chunk by `BLAKE3(plaintext)`. When tebako
packages two versions of the same app, the shared files deduplicate
automatically at the drop level — no explicit delta logic needed.

### 4. Per-content-class compression

LimniFS's seine classifier routes each chunk to the optimal codec:
ZSTD for source code, LZ4 for binaries, Brotli for text, store for
already-compressed assets. This gives better overall ratio than DwarFS's
single-codec-per-block approach.

### 5. In-memory read path

LimniFS's slab reader works on any `&[u8]` — a memory-mapped file, a
heap-allocated buffer, or an embedded resource. For tebako, the `.lim`
image can be appended to the executable binary and memory-mapped at
runtime, with zero-copy access to individual drops.

## Proposed adapter interface

The `libtfs` adapter trait needs to expose:

```rust
trait OffsAdapter {
    /// Open an OFFS image from a byte slice (the embedded resource).
    fn open(data: &[u8]) -> Result<Self, OffsError>;

    /// List entries in a directory.
    fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, OffsError>;

    /// Read a file's contents into a Vec.
    fn read_file(&self, path: &str) -> Result<Vec<u8>, OffsError>;

    /// Stat a file or directory.
    fn stat(&self, path: &str) -> Result<FileStat, OffsError>;

    /// Check if a path exists.
    fn exists(&self, path: &str) -> bool;
}
```

LimniFS already provides all of these through `limnifs-core`:

```rust
use limnifs_core::{ManifestCursor, parse_manifest_header, parse_metadata_blob, ...};

struct LimnifsAdapter {
    blob: MetadataBlob,
    slab: SlabView<'static>,
}

impl OffsAdapter for LimnifsAdapter {
    fn open(data: &[u8]) -> Result<Self, OffsError> {
        let mut cursor = ManifestCursor::new(data);
        let header = parse_manifest_header(&mut cursor)?;
        let meta_ref = parse_metadata_reference(&mut cursor)?;
        // ... parse inline metadata + slab index
        Ok(Self { blob, slab })
    }

    fn read_file(&self, path: &str) -> Result<Vec<u8>, OffsError> {
        let inode = self.blob.resolve_path(path)?;
        let handle = inode.content_handle;
        self.slab.read_drop(&handle)  // decompresses on the fly
    }

    // ...
}
```

The key advantage: LimniFS's read path is **zero-allocation for inline
drops** and **on-demand decompression for slab-backed drops**. The
runtime pays no upfront cost to "mount" the filesystem — it reads and
decompresses drops lazily, only when the application accesses them.

## Integration approach

### Phase 1: LimniFS as a read-only OFFS backend

1. Add `limnifs-core` as a Cargo dependency in tebako's Rust runtime
   component.
2. Implement `OffsAdapter for LimnifsAdapter`.
3. Add a CLI flag: `tebako press --format limnifs` (default: `dwarfs`).
4. The runtime detects the OFFS format by magic bytes:
   - DwarFS: starts with the DwarFS magic.
   - LimniFS: starts with `LMFS` (0x4C4D4653).

### Phase 2: Format negotiation

The tebako runtime probes the OFFS magic bytes at startup and selects
the matching adapter. No user action needed — a tebako package built
with `--format limnifs` automatically uses the LimniFS adapter at
runtime.

### Phase 3: Shared runtime

Since LimniFS is pure Rust, a tebako runtime built with the LimniFS
adapter doesn't need any C++ libraries. The entire OFFS stack —
compression, content addressing, metadata, slab reading — is Rust.
This dramatically simplifies the tebako runtime build.

## Migration path

Existing tebako packages using DwarFS continue to work unchanged.
New packages can opt into LimniFS with `--format limnifs`. The two
formats coexist; the runtime auto-detects.

For users who want to convert existing DwarFS packages to LimniFS:
```bash
# Extract the DwarFS package to a temp dir
tebako unpress old-package.bin /tmp/extracted/
# Re-pack with LimniFS
tebako press /tmp/extracted/ new-package.bin --format limnifs
```

## What tebako gets

| Feature | DwarFS | LimniFS |
|---|---|---|
| Language | C++ | Pure Rust |
| Build deps | libdwarfs + 6 C libs | None |
| Cross-compile | Full C++ toolchain per target | `cargo build --target` |
| Create speed | baseline | 1.6x faster |
| Extract speed | baseline | 2.3x faster |
| Content addressing | No | Yes (BLAKE3) |
| Deduplication | Block-level | Content-level (BLAKE3) |
| Compression | Per-block, single codec | Per-content-class, 7 codecs |
| Air-gapped builds | No (C deps) | Yes |

## What LimniFS gets

Real-world adoption by a mature packaging tool, validation against
diverse workloads (Ruby gems, Python packages, Node.js modules), and
feedback on the read-path performance characteristics that matter for
embedded runtimes.

## Next steps

1. Review the adapter trait proposal — does it match `libtfs`'s design?
2. Prototype: build a minimal `LimnifsAdapter` in a tebako fork.
3. Benchmark: compare DwarFS vs LimniFS OFFS performance on a real
   tebako package (e.g., a Rails app with 500 gems).
4. Ship: merge the adapter into tebako main behind `--format limnifs`.

We're happy to contribute the adapter implementation and help with
integration testing.
