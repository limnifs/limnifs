# Changelog

All notable changes to LimniFS are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.2.1] — 2026-08-04

7 PRs landed since v0.2.0 (#137 → #143). Major themes: RW image
correctness, codec framework maturation, DRY refactors, omnizip
0.14.1 upgrade.

### Added

- **Codec tunables wiring** — `CodecTunables` struct +
  `Codec::compress_with_tunables` trait method. PPMd7/8 order +
  memory budget, Brotli quality, ZSTD level, Bzip2 block size now
  actually reach the codecs via the parallel writer. (PR #137)
- **Real `RwImage::open/commit/turnover`** — `RwImage::open` now
  parses manifest + mmaps slabs + builds path index. `commit` and
  `turnover` materialize the live tree and rebuild. (PR #137)
- **BCJ composite codecs** — `CODEC_BCJ_X86_LZ4` (0x20),
  `CODEC_BCJ_X86_ZSTD` (0x21), `CODEC_BCJ_ARM64_LZ4` (0x23),
  `CODEC_BCJ_ARM64_ZSTD` (0x24). Proven ratio win on synthetic
  x86-call fixtures. (PR #138)
- **Hot slab LRU cache** — `CachedSlabStore` wraps `SlabStore` and
  caches decoded plaintext by `DropId`. In-house LRU (no dep).
  (PR #138)
- **LZ4 HC** — codec id `0x13 = CODEC_LZ4_HC`. Real hash-chain
  match finder via omnizip-lz4 0.14.1 (proposal #1 accepted).
  Added to `max-ratio` tournament. (PR #141, #142)
- **Atomic image swap** — `RwImage::write_artifact` writes to
  `<image>.new/` then renames into place (sidecar → slabs →
  manifest ordering). (PR #142)
- **Crash recovery on open** — `RwImage::open` cleans up stale
  `<image>.new/` from interrupted previous commit. (PR #143)
- **Executable categorizer routing** — `ExecutableCategorizer`
  detects ELF / PE / Mach-O magics and routes x86_64 / aarch64
  architectures to BCJ composite codecs. (PR #142)
- **Composite-codec shared helper** —
  `limnifs_core::codec::composite::{filter_then_compress,
  decompress_then_filter}`. 7 composites now share one pipeline
  (DRY). (PR #139)
- **Chunker trait** — `limnifs_write::chunker::Chunker` trait +
  impl for `FastCDC`. New chunker = one impl. (PR #139)
- **Live tree walker** — `limnifs_core::live_tree` module with
  `LiveTreeSink` trait, `walk_live_tree` walker, `FilesystemSink`,
  `ParallelExtractSink`, `DropIdCollectorSink`, and canonical
  `file_plaintext`. Fixes sub-drop addressing bug. (PR #140, #143)

### Changed

- **omnizip 0.13.1 → 0.14.1** — picks up LZ4 HC, LZMA optimal
  parser, ZPAQ 7-model portfolio + warmup word model, multi-byte
  FSE (2-state interleave), SIMD Huffman Phase 1. (PR #141)
- **`compress_with_options` is now a thin shim** around
  `compress_with_tunables` — single dispatch path (DRY).

### Documentation

- 17 TODO specs filed under `TODO.impl/` covering the 2026
  throughput roadmap (CDC, dedup, compression, indexing, IO, RW).
- 8 omnizip-rs upstream proposals at `docs/omnizip-proposals/` —
  5 accepted (LZ4 HC, ZPAQ word, multi-byte FSE, LZMA optimal
  parser, plus partial SIMD Huffman), 3 partial.

### Test count

538 → **561** (+23 across all features).

## [0.2.0] — 2026-07-29

Initial public release. Wire format pivot: custom everywhere,
Merkle B-tree, `.lim` extension, multi-file spec.

[0.2.1]: https://github.com/limnifs/limnifs/releases/tag/v0.2.1
[0.2.0]: https://github.com/limnifs/limnifs/releases/tag/v0.2.0
