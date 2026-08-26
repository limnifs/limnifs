# tebako: upgrading to the new LimniFS read path

For tamatebako/tebako#464 and limnifs#192 — how to take advantage of
the v0.3.0 read path, and what changes. This is the integration
guide; the API reference is [docs/read-api.md](read-api.md).

## TL;DR

- Replace tebako's consumer-side one-drop memo with
  `limnifs_core::read::{ImageReader, FileReader}` — the memoized,
  bounded, seek-aware reader is now in the library.
- **Rebuild cached images.** There is exactly one image format now;
  images written by limnifs ≤ 0.2.64 are NOT readable by 0.3.0+
  (alpha decision — see below). tebako's cached runtimes must be
  re-packed once.
- Expect mounted-read performance to stop mattering: a cold 8 KiB
  window on a 19.5 MiB file drops from ~58 ms of decode to ~0.6 ms
  (98×), and repeat windows are memcpy-class.
- The incompatible image-format change is signaled by the **0.3.0 minor bump**. Do not use 0.2.x images with 0.3.x readers; re-pack once.

## What changed in limnifs

| Area | ≤ 0.2.64 | 0.2.65 |
|---|---|---|
| Drop records | 49 B (no flags) | 50 B, trailing `flags` byte |
| Large drops | monolithic codec stream | seekable container: independent 256 KiB frames + `LMSK` footer (general codecs, > 1 MiB) |
| Windowed read | whole-drop decode per window | covering-frames-only decode (binary search on the footer) |
| Caching | none on the read path | SIEVE drop cache (64 MiB) + SIEVE frame cache (32 MiB), both `Arc`-shared |
| Slab lookups | re-parsed per read | parsed once at open, O(1) lookups, mmap'd slabs |
| Public API | raw `SlabStore` + codec calls | `ImageReader` / `FileReader` / `extract_file` |

`DropId` is still `BLAKE3(plaintext)` — identity, dedup, and Merkle
verification are unchanged. Trained-dictionary drops and whole-stream
codecs (FLAC/RICEPP) stay monolithic; the `max_drop_size` writer knob
(4 MiB default) bounds those by chunking instead.

## How to use it

```rust
use limnifs_core::read::{ImageReader, ReadConfig};
use std::io::Read;

// Open once per mounted image. Slabs are mmap'd; caches sized by config.
let reader = ImageReader::open(manifest_path.into(), ReadConfig::default())?;

// Mount hot path: positional reads decode only the covering frames.
let file = reader.file("/usr/bin/app")?;
let n = file.read_at(offset, &mut buf)?;   // std::io::Read also impl'd

// Bulk extraction: drops are independent — decode them on rayon.
limnifs_core::read::extract_file(&manifest, "/usr/bin/app", &mut w,
    ReadConfig { parallel_decode: true, ..ReadConfig::default() })?;
```

Tuning (all optional):

- `ReadConfig::cache_bytes` / `cache_entries` — full-drop cache
  (small drops land here).
- `ReadConfig::frame_cache_bytes` (default 32 MiB) — seekable frame
  cache; raise it if your working set is many hot large files.
- `seekable::frames_decoded()` — process-wide frame-decode counter;
  useful in tebako's tests to assert bounded work.

If tebako packs images itself: defaults already emit containers for
large general-codec drops. `defaults.seekable_drops = false` opts out
for maximum ratio (costs windowed reads); `defaults.max_drop_size`
(default 4 MiB, `0` = unlimited) bounds whole-file drops by chunking.

## The numbers (19.5 MiB file, 8 KiB windows — your #464 scenario)

`limnifs-bench readcompare`, same fixture packed monolithic vs
seekable:

```
metric                     monolithic     seekable      delta
first window                 58332 us       595 us     0.01x
cold windowed               0.1 MB/s    11.4 MB/s    83.03x
warm windowed              9099 MB/s    6383 MB/s     0.70x
sequential extract          336 MB/s     368 MB/s     1.10x
image size                 14.52 MiB    14.49 MiB     1.00x
cold work per 8 KiB window: whole 19.5 MiB drop  vs  1.03 × 256 KiB frames
```

The old failure mode — ~2,500 windows × 19.5 MiB ≈ 48 GiB of decode —
is now ~2,500 × 256 KiB ≈ 640 MiB worst case, and effectively ~2,500
frame-cache hits after the first touch of each frame.

## Migration steps

1. Bump `limnifs` to ≥ 0.2.65 (crates.io).
2. Delete the tfs-adapter one-drop memo — `CachedSlabStore` (behind
   `ImageReader`) supersedes it with bounded, scan-resistant policy.
3. Route `pread`-style access through `FileReader::read_at`.
4. Re-pack tebako's cached images once (old layout is unreadable);
   the pack side is unchanged apart from emitting containers.
5. Optional: assert `frames_decoded()` deltas in tebako's read tests
   to lock the bounded behavior in CI on your side too.
