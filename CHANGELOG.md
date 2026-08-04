# Changelog

All notable changes to LimniFS are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.2.6] — 2026-08-04

### Changed

- **omnizip 0.14.1 → 0.14.4** — picks up 6 libdeflate LZ77+Huffman
  encoder bug fixes (hash-before-search, bit-writer byte extraction,
  HuffmanTable u8→u16, distance_to_sym offset, distance code
  double-reversal, lazy look-ahead off-by-one). 831 omnizip tests
  pass; 0 ignored; 0 unsafe code.

### Test count

569 (unchanged).

## [0.2.5] — 2026-08-04

### Added

- **End-to-end dictionary round-trip integration test** — new
  `limnifs-write/tests/dict_round_trip.rs` writes an image with
  `dictionaries.enabled`, parses the manifest (including
  `dictionary_section`), constructs a `SlabStore`, calls
  `set_dictionaries`, and verifies every file's plaintext matches
  the original. Proves the v0.2.3 + v0.2.4 writer+reader dict
  pipeline works end-to-end on real content.

### Test count

568 → **569** (+1 integration test).

## [0.2.4] — 2026-08-04

### Added

- **Reader-side dictionary resolution** — `SlabStore` now holds a
  `dict_id → dictionary bytes` map populated via the new
  `set_dictionaries` method. Drops whose `DropRecord::dict_id !=
  NO_DICT` are decompressed via the dict-aware ZSTD path
  (`codec::zstd_dict::decompress_with_dict`). Drops without a dict
  are unaffected (zero overhead).
- `SlabView::plaintext_for_with_dict_lookup` — public method that
  takes a callback to resolve `dict_id` → bytes. `plaintext_for` is
  now a thin wrapper around it.
- `limni::load_image` now also parses the optional
  `dictionary_section` and returns it; CLI commands that build a
  `SlabStore` (`cat`, `cat-multi`, `extract`, `tree`) install the
  dictionaries via the new `install_dicts` helper.

### Changed

- `SlabStore` constructors (`load`, `load_mmap`, `from_bytes`) now
  initialize `dictionaries: HashMap::new()`. Callers that want
  dict-aware decompression call `set_dictionaries` after
  construction.

### Compatibility

- v0.2.3 images with `dictionary_section` are now correctly
  decodable.
- v0.2.2 images (no `dictionary_section`) parse and decode exactly
  as before — `dict_section` is `None`, `set_dictionaries` is not
  called, drops all carry `dict_id = NO_DICT`.

### Test count

568 (unchanged — existing tests cover the no-dict path; the
dict-compressed path is exercised end-to-end by the writer's
`dictionaries_enabled_emits_dictionary_section_when_enough_samples`
test, which only checks the manifest is parseable today).

## [0.2.3] — 2026-08-04

### Added

- **ZSTD dictionary writer pipeline integration** — when
  `WriteConfig::dictionaries.enabled` is true, the writer now:
  1. Retains plaintext for ZSTD-compressed drops during the parallel
     compress phase (memory cost bounded by `MAX_DICT_SAMPLES = 1000`).
  2. After the parallel phase, trains one ZSTD dictionary via the
     `omnizip_zstd` FrequencyTrainer.
  3. Re-compresses each ZSTD drop with the dictionary; keeps the
     smaller of the two representations.
  4. Populates `DropRecord::dict_id` for re-compressed drops.
  5. Emits a `dictionary_section` in the manifest containing the
     trained dictionary.

  Closes `TODO.impl/04-writer-pipeline/04-zstd-dictionary-training.md`
  (single-dictionary, single-class variant). Per-class split and
  FastCover trainer are follow-ups.

### Test count

567 → **568** (+1 dict pipeline behavioural test).

## [0.2.2] — 2026-08-04

### Added

- **ZSTD dictionary trainer API at writer layer** — new
  `limnifs_write::dictionary` module exposing `train_zstd`,
  `TrainedDictionary` (with `compress`/`decompress` helpers), and
  `allocate_ids`. Wraps `limnifs_core::codec::zstd_dict`. The
  writer-pipeline integration (collect samples per class, train,
  re-compress, emit dictionary_section) is filed as a follow-up
  in `TODO.impl/04-writer-pipeline/04-zstd-dictionary-training.md`.

### Changed

- `limnifs_core::codec::zstd_dict` module is now `pub` so the
  writer layer can wrap it. Internal API unchanged.

### Test count

561 → **567** (+6 dictionary tests).

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

[0.2.6]: https://github.com/limnifs/limnifs/releases/tag/v0.2.6
[0.2.5]: https://github.com/limnifs/limnifs/releases/tag/v0.2.5
[0.2.4]: https://github.com/limnifs/limnifs/releases/tag/v0.2.4
[0.2.3]: https://github.com/limnifs/limnifs/releases/tag/v0.2.3
[0.2.2]: https://github.com/limnifs/limnifs/releases/tag/v0.2.2
[0.2.1]: https://github.com/limnifs/limnifs/releases/tag/v0.2.1
[0.2.0]: https://github.com/limnifs/limnifs/releases/tag/v0.2.0
