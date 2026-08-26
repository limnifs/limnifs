# The LimniFS read path — API guide

How to read `.lim` images efficiently from Rust, and what the reader
does for you. Covers `ImageReader` / `FileReader` (the public library
API), the mount path, the caching and seekable-container machinery
underneath them, and the tuning knobs that matter.

## Quick start

```rust
use limnifs_core::read::{ImageReader, ReadConfig};
use std::io::Read;

let reader = ImageReader::open(std::path::Path::new("app.lim"), ReadConfig::default())?;
let mut file = reader.file("/usr/bin/app")?;
let mut buf = Vec::new();
file.read_to_end(&mut buf)?;
```

`ImageReader::open` parses the manifest, loads (or mmaps) the slab
sidecars listed in its index, and builds the drop cache. Missing or
corrupt slabs fail at `open`, never mid-read.

## The API surface

| Item | What it does |
|---|---|
| `ImageReader::open(path, config)` | Open an image from disk. |
| `ImageReader::from_parts(bytes, dir, config)` | Open from in-memory manifest bytes plus the sidecar directory. |
| `reader.file(path)` | Resolve `/separated/path` to a `FileReader`. Errors on missing paths and non-regular inodes. |
| `FileReader::read_at(offset, buf)` | Positional read — decode only what the window touches. |
| `impl Read for FileReader` | Sequential streaming; tracks a cursor. |
| `FileReader::size()` | File length from the metadata blob. |
| `extract_file(image, path, writer, config)` | One-shot extract, optionally with `parallel_decode` (drops are independent — rayon decodes them concurrently, then writes in order). |
| `reader.cache_stats()` | SIEVE cache counters (hits/misses/evictions/bypasses, bytes vs budgets). |

### ReadConfig

| Field | Default | Meaning |
|---|---|---|
| `cache_bytes` | 64 MiB | Byte budget of the decoded-drop cache. |
| `cache_entries` | 1024 | Entry cap (whichever bound hits first evicts). |
| `parallel_decode` | false | `extract_file` decodes drops on rayon. |
| `frame_cache_bytes` | 32 MiB | Byte budget of the seekable frame cache. |

## What happens under a windowed read

The old reader materialized a whole drop's plaintext for every 8 KiB
window — reading a 19.5 MiB file through FUSE re-decompressed ~48 GiB
(limnifs#192). The current read path bounds the work instead:

1. **Slice map first.** `read_at` walks only the slices that intersect
   `[offset, offset+len)` — non-covering drops are never touched.
2. **Seekable containers.** Drops over 1 MiB written with general
   codecs are stored as independent ~256 KiB codec frames + a footer
   index. A windowed read binary-searches the footer and decompresses
   only the covering frame(s): a cold 8 KiB window on a 19.5 MiB drop
   costs one 256 KiB frame, not 19.5 MiB. Decoded frames live in their
   own SIEVE cache (`frame_cache_bytes`, 32 MiB default) so repeat
   windows in a hot frame are refcount bumps; footers are memoized.
3. **SIEVE drop cache.** Non-seekable (small) drops are cached whole
   as `Arc<[u8]>` — hits are refcount bumps. SIEVE (USENIX ATC '24)
   eviction is O(1) and scan-resistant; both an entry cap and a byte
   budget apply, and a drop larger than the whole budget bypasses the
   cache rather than evicting the working set. Slab record tables are
   parsed once at open, so every lookup is O(1) — no re-parse per
   read — and slab sidecars are mmap'd (pages enter RSS on demand).
4. **`drop_id` never changes.** A container still hashes the full
   plaintext (BLAKE3), so dedup and Merkle verification are unaffected.

`limnifs_core::seekable::frames_decoded()` exposes a process-wide
frame-decode counter — useful for asserting bounded work in tests
(the conformance and round-trip suites use it).

## Bounded-output drops (writer knob)

`WriteConfig::defaults.max_drop_size` (default 4 MiB, `0` =
unlimited) caps the plaintext of any single whole-file drop: files
above the cap fall back to FastCDC chunking + codec tournament, so
the decompressed unit behind a random access is bounded by
construction (the EROFS fixed-output-pcluster idea). The
`skip_chunking` (max-write) profile is exempt — whole-file is its
speed contract — and large drops there are seekable containers
instead.

Recipe — keep shared libraries raw (P3 from the audit):

```toml
[[categorizer]]
name = "shared-libs-store"
extensions = ["so", "dylib", "dll"]
codec = "store"
```

Shared libraries are usually already-compressed payloads; `store`
avoids burning CPU re-deflating them. Categorizer claims respect
`max_drop_size`: a `.so` larger than the cap falls through to
chunking (where the classifier stores the chunks anyway).

## Seekable drop containers (writer side)

Writers emit drops over 1 MiB (general codecs) as `LMSK` containers,
flagged in the drop record's trailing `flags` byte (bit0 = SEEKABLE):

```text
container := frame* footer
frame      := independently-decodable codec stream covering one
              contiguous uncompressed sub-range (256 KiB target)
footer     := per frame: u32 uncomp_len, u32 comp_len,
              then a fixed 10-byte tail read back-to-front:
              "LMSK" (4) + u16 version (2) + u32 frame_count (4)
```

There is exactly one slab format and one drop-record layout
(50 bytes) — alpha software carries no version history; images from
earlier versions are not readable. General stream codecs
(LZ4, ZSTD, XZ, Brotli, DEFLATE, Snappy, bzip2, PPMd, ...) emit
containers; whole-stream codecs (FLAC, RICEPP) and trained-dictionary
drops stay monolithic and rely on `max_drop_size` for bounding.
`defaults.seekable_drops = false` opts out entirely (maximum ratio;
`max-ratio` profile sets it).

## FUSE mount

`limni mount` uses the same `CachedSlabStore` and windowed read path,
so mounts get the same bounds: 8 KiB FUSE reads decode at most one
seekable frame or one cached small drop. `limni` prints cache stats on
unmount for a quick sanity check of hit rates.
