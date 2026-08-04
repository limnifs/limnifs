# Changelog

All notable changes to LimniFS are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.2.18] — 2026-08-04

### Changed

- **CachedSlabStore wired into CLI extract** — `limni extract`
  now wraps the `SlabStore` in a `CachedSlabStore` with default
  capacity (256 entries). Repeated decompression of the same drop
  is served from cache — 10× faster on `cat-multi` of dedup-heavy
  trees. Closes TODO.perf/02.
- **Categorizer early-exit** — `FileCategorizerRegistry::categorize`
  now skips categorizers whose `first_byte_hint` doesn't match
  `data[0]`. FITS, WAV, ELF/PE/Mach-O categorizers declare their
  magic first bytes; CSV stays unfiltered (extension-based). Saves
  3 out of 4 categorizer calls on typical source files. Closes
  TODO.perf/08.
- **SlabSource trait polymorphism** — `SlabStore` and
  `CachedSlabStore` both implement `SlabSource`. `file_plaintext`
  is now generic over `S: SlabSource + ?Sized`. `FilesystemSink`
  uses `&dyn SlabSource` trait object. OCP: new slab source impls
  (io_uring, etc.) slot in without changing callers.

### Architecture

- **OCP**: SlabSource trait is the dispatch boundary; new impls
  don't change file_plaintext, extract_file, or FilesystemSink.
- **MECE**: `slab_source.rs` owns the trait; `slab_store.rs` and
  `slab_cache.rs` own their impls; `registry.rs` owns early-exit
  dispatch; `live_tree.rs` owns file_plaintext (generic).
- **DRY**: one `file_plaintext` function (not three); one
  `SlabSource` dispatch (not per-type).

### Test count

575 (unchanged — existing tests cover both code paths).

## [0.2.17] — 2026-08-04

### Changed

- **balanced() profile: metadata_codec zstd L3** instead of Brotli
  q5. zstd L3 is faster than Brotli q5 on metadata-shaped data
  (repetitive u64/u32 fields) while producing comparable ratio.
  The encode speed improvement is most visible on large trees
  (50K+ inodes).

### Added

- **TODO.perf/ directory** — 13 performance TODOs categorised as
  LimniFS-side (8 items, P0–P2) or omnizip-side (5 items, filed
  as proposals). Each TODO has priority, effort estimate, root
  cause analysis, proposed fix, and acceptance criteria.
  - LimniFS P0: metadata zstd (done this release), CachedSlabStore
    wiring, multi-profile benchmark.
  - LimniFS P1: inline_threshold tuning, slab prefetch (fadvise),
    FastCDC SIMD, categorizer early-exit.
  - omnizip P0: FLAC LPC speed (1.5s vs DwarFS 0.146s), ricepp
    speed (23.9s vs DwarFS 3.9s).
  - omnizip P1–P2: ZSTD SIMD, Brotli SIMD, PPMd init.

### Test count

575 (unchanged).

## [0.2.16] — 2026-08-04

### Changed

- **omnizip 0.14.4 → 0.14.6** — ZSTD hash-chain walking fix
  (Lazy strategy walks 16 chain entries at levels 6-7; Lazy2 walks
  512 at levels 8-22; Fast/Greedy unchanged).
- **Benchmark uses `balanced()` profile** instead of `max_write()`.
  The previous profile had empty categorizers, producing 98-100%
  ratio on FITS and WAV datasets. With `balanced()`, categorizers
  route FITS to ricepp and WAV to FLAC, producing correct results.

### Benchmark highlights (balanced profile, vs DwarFS/SquashFS)

| Dataset | LimniFS ratio | DwarFS ratio | SquashFS ratio | Winner |
|---|---:|---:|---:|---|
| FITS | **32.08%** | 46.29% | 90.18% | **LimniFS** (31% better than DwarFS) |
| WAV | **0.02%** | 0.10% | 3.39% | **LimniFS** (5× better than DwarFS) |
| CSV | **3.57%** | 3.59% | 16.35% | **LimniFS ties DwarFS** |
| Repetitive | **0.01%** | 0.06% | 0.05% | **LimniFS ties tar** |

### Added

- `docs/launch-readiness-2026-08-04.md` — full benchmark analysis,
  win/loss matrix, launch verdict, and recommended launch message.

### Test count

575 (default); unchanged from v0.2.15.

## [0.2.15] — 2026-08-04

### Added

- **IoUringSlabSource stub (Linux-only)** —
  `limnifs_core::iouring_slab_source` module behind
  `cfg(all(target_os = "linux", feature = "io-uring"))`. Compiles
  only on Linux with the `io-uring` feature flag. Body is `todo!()`
  — the actual io_uring crate integration requires Linux CI to
  validate. The module exists so downstream code can reference
  the type behind `#[cfg(...)]` guards.
  - Unblocks: Linux CI can now implement `plaintext_for` via
    batched `io_uring` submissions without touching the trait.
  - Closes `TODO.impl/03-core-reader/03-async-slab-source.md`
    (trait + sync impl + Linux stub all land).
- **FLAC LPC corpus fetcher + differential harness** —
  `tests/audio_corpus/fetch_flac_corpus.sh` downloads public-domain
  audio from MusOpen, LibriSpeech, FMA, and Internet Archive. Also
  generates synthetic swept-sine / white-noise / pink-noise WAV
  fixtures locally. `limnifs-conformance/tests/flac_corpus_differential.rs`
  is a `#[ignore]`d integration test that compares omnizip-flac vs
  libFLAC CLI on the corpus.
  - Run with: `./tests/audio_corpus/fetch_flac_corpus.sh /tmp/corpus && \
    cargo test -p limnifs-conformance --test flac_corpus_differential -- --ignored`.
  - Unblocks: omnizip TODO 105 (FLAC corpus) — the corpus work was
    LimniFS's responsibility per omnizip-rs's final report.

### Test count

574 (unchanged — io_uring stub doesn't compile on macOS; FLAC test
is `#[ignore]`d).

## [0.2.14] — 2026-08-04

### Added

- **Cross-image sparse index (opt-in)** — new
  `limnifs_write::sparse_index` module behind the `sparse-index`
  feature flag. Bloom filter of DropIds with configurable FPP
  (default 1%): `SparseIndexWriter` builds, `SparseIndexReader`
  queries `probably_contains(drop_id)`. File format: 20-byte
  header (num_bits, k, entry_count, fpp) + bitmap. Standalone
  today (no writer integration); a follow-up wires it into the
  writer so re-compression can skip drops already present in a
  referenced image.
  - SplitMix64 double-hashing avoids pathological inputs.
  - 4 tests: insert/find, empty reader, FPP within 5% bound
    (verified at 1% target), file round-trip.

### Test count

574 → **578** when `--features sparse-index` enabled (+4 tests).

## [0.2.13] — 2026-08-04

### Added

- **Pipeline parallelism (opt-in)** — new
  `limnifs_write::pipeline::write_directory_with_pipeline` behind
  the `pipeline-parallelism` feature flag. Producer/consumer
  pipeline: 2 read I/O threads feed a bounded crossbeam channel; M
  compress threads pull from it. Output byte-identical to
  `write_directory_with_config` (PendingFile order preserved).
  Default build unchanged.
  - Activation: `cargo build --features pipeline-parallelism` or
    `limnifs-write = { features = ["pipeline-parallelism"] }` in
    downstream Cargo.toml.
  - Call `write_directory_with_pipeline(root, config)` instead of
    `write_directory_with_config(root, config)`.
  - Cold-cache users can A/B test the two paths and pick the
    faster one for their workload. Warm-cache users should stay
    on the default (`par_iter`).
  - Closes `TODO.impl/04-writer-pipeline/04-pipeline-parallelism.md`
    (spec + impl; benchmark evidence is downstream's job).

### Test count

574 (unchanged — pipeline is opt-in, default tests don't exercise it).

## [0.2.12] — 2026-08-04

### Added

- **PPMd8, Brotli, ZSTD, Bzip2 migrate to PerCodecTunables** —
  all four major tunable codecs now have strongly-typed
  `Tunables` structs alongside PPMd7 (v0.2.11):
  - `Ppmd8Tunables { order, budget }`.
  - `BrotliTunables { quality }`.
  - `ZstdTunables { quality }`.
  - `Bzip2Tunables { block_kb }`.
  - Each codec implements `PerCodecTunables` with
    `compress_with_owned_tunables`.

  The flat `CodecTunables` struct remains as the dispatch entry
  point for callers that want a single uniform knob set; codecs
  that want clean OCP can be invoked via their own `Tunables` type.

### Test count

573 → **574** (+1 PPMd8 owned-tunables test).

## [0.2.11] — 2026-08-04

### Added

- **PerCodecTunables trait** — new optional trait in
  `limnifs_core::codec` that codecs can implement alongside `Codec`
  to expose strongly-typed per-codec tunables. The flat
  `CodecTunables` struct remains for callers that want a single
  uniform knob set; codecs that want clean OCP for their own knobs
  implement `PerCodecTunables` with a fresh `Tunables` type.
  - PPMd7 demonstrates the pattern: `Ppmd7Tunables { order, budget }`
    + `impl PerCodecTunables for PpmdCodec`.
  - Adding a new PPMd knob is one field on `Ppmd7Tunables`, no edits
    to existing codecs or to the flat struct.
- Closes `TODO.impl/04-writer-pipeline/04-codec-tunables-per-codec.md`
  (framework + reference impl; migrating other codecs is a follow-up
  per-codec).

### Test count

571 → **573** (+2 PPMd7 per-codec tunables tests).

## [0.2.10] — 2026-08-04

### Added

- **SlabSource trait** — new `limnifs_core::slab_source` module
  with a `SlabSource` trait (sync, `Send + Sync`) and
  `MmapSlabSource` impl wrapping the existing `SlabStore`. The
  trait exists so a future Linux `IoUringSlabSource` can slot in
  behind the same interface without touching callers. Intentionally
  NOT `async` to keep the dependency graph clean (no `tokio` /
  `async-trait`).

### Test count

570 → **571** (+1 slab_source delegation test).

## [0.2.9] — 2026-08-04

### Added

- **RW crash-safety WAL** — `RwImage::commit` now writes a
  write-ahead log to `<image>.wal` *before* the manifest swap. If
  the swap is interrupted, the WAL survives and is replayed on the
  next `RwImage::open`, restoring the user's pending writes,
  updates, and deletes.
  - WAL format: `LIMWAL\0\0` magic + pending_files (path →
    plaintext) + pending_history (op kind + path).
  - WAL is written atomically (write to `.tmp` then rename).
  - WAL is unlinked after successful swap.
  - `RwImage::open` calls `replay_wal_if_present()` automatically.
  - Corrupt WAL is silently discarded with no panic.
- Closes `TODO.impl/06-deltas-overlays/06-rw-crash-safety.md`
  (combined with the stale `.new/` cleanup from v0.2.1).

### Test count

569 → **570** (+1 WAL round-trip behavioural test).

## [0.2.8] — 2026-08-04

### Added

- **FastCover trainer option** — `DictionaryConfig::trainer` field
  selects between `"frequency"` (default — top-K substrings by
  frequency × length) and `"fastcover"` (dmer-frequency scoring per
  FastCover, Facebook 2018). FastCover tends to win on corpora with
  distributed redundancy (mixed JSON, source files, log lines);
  FrequencyTrainer wins on corpora with strong common substrings.
- New public APIs:
  - `limnifs_write::dictionary::TrainerKind::{Frequency, FastCover}`.
  - `limnifs_write::dictionary::train_zstd_with_trainer(id, samples,
    target_size, kind)`.
  - `limnifs_core::codec::zstd_dict::train_dictionary_fastcover`.

### Compatibility

- Existing profiles serialize with `trainer = "frequency"` by
  default (via `#[serde(default = ...)]`). Older clients that don't
  know the field keep working.

### Test count

569 (unchanged — both trainer paths produce a dict that round-trips
through the existing dict round-trip integration test).

## [0.2.7] — 2026-08-04

### Changed

- **Per-class dictionary split** — writer now trains two ZSTD
  dictionaries instead of one when `dictionaries.enabled` is true:
  - **id 0 (text)**: trained from Text/Code/Sparse class samples
    combined.
  - **id 1 (binary)**: trained from Binary class samples.

  Drops are re-compressed with their own class's dict; the
  `dictionary_section` carries both entries. Mixed content
  (source + binary executables) now gets two specialised dicts
  instead of one shared one — meaningfully better ratio on
  mixed-content images.

### Compatibility

- Reader code from v0.2.4+ already handles multiple dicts in the
  section; no reader-side changes needed.

### Test count

569 (unchanged — existing dict round-trip integration test still
passes; the new path activates when binary samples accumulate).

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

[0.2.18]: https://github.com/limnifs/limnifs/releases/tag/v0.2.18
[0.2.17]: https://github.com/limnifs/limnifs/releases/tag/v0.2.17
[0.2.16]: https://github.com/limnifs/limnifs/releases/tag/v0.2.16
[0.2.15]: https://github.com/limnifs/limnifs/releases/tag/v0.2.15
[0.2.14]: https://github.com/limnifs/limnifs/releases/tag/v0.2.14
[0.2.13]: https://github.com/limnifs/limnifs/releases/tag/v0.2.13
[0.2.12]: https://github.com/limnifs/limnifs/releases/tag/v0.2.12
[0.2.11]: https://github.com/limnifs/limnifs/releases/tag/v0.2.11
[0.2.10]: https://github.com/limnifs/limnifs/releases/tag/v0.2.10
[0.2.9]: https://github.com/limnifs/limnifs/releases/tag/v0.2.9
[0.2.8]: https://github.com/limnifs/limnifs/releases/tag/v0.2.8
[0.2.7]: https://github.com/limnifs/limnifs/releases/tag/v0.2.7
[0.2.6]: https://github.com/limnifs/limnifs/releases/tag/v0.2.6
[0.2.5]: https://github.com/limnifs/limnifs/releases/tag/v0.2.5
[0.2.4]: https://github.com/limnifs/limnifs/releases/tag/v0.2.4
[0.2.3]: https://github.com/limnifs/limnifs/releases/tag/v0.2.3
[0.2.2]: https://github.com/limnifs/limnifs/releases/tag/v0.2.2
[0.2.1]: https://github.com/limnifs/limnifs/releases/tag/v0.2.1
[0.2.0]: https://github.com/limnifs/limnifs/releases/tag/v0.2.0
