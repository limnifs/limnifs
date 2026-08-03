# 04 — 2026 throughput & ratio roadmap

- **Status:** in_progress
- **Phase:** 1–2 (mixed)
- **Depends on:** 04-deepening-compactor, 03-drop-store-reader, 06-turnover
- **Design refs:** §6 (pipeline), §16 (open questions 1–2), 2026 academic CDC/dedup/compression SOTA

## Goal

Catalogue every 2026-vintage throughput and ratio technique applicable to
a content-addressed, RW, Merkle-rooted image filesystem, score each
against LimniFS's current state, and assign a priority. This doc is the
index — every named item is filed as its own TODO with a status
self-check; the index keeps the MECE picture.

## Priority legend

- **P0** — ships next; user-visible win or unblocks something else.
- **P1** — ships this cycle; ratio or throughput win, no new architecture.
- **P2** — research / future; needs spec or external dep before it can land.

## 1. Content-defined chunking (CDC)

| Technique | Source | Status in LimniFS | Priority |
|---|---|---|---|
| FastCDC (Xia 2016, normalized chunking) | `chunker::FastCDC` | done | — |
| Gear-hash + SIMD scan | FastCDCv2 / BFBC (2022-2024) | not wired | **P1** |
| Leap-based parallel CDC (LPC-DC) | Li 2023 | not wired | P2 |
| Adaptive-entropy CDC (AE-CDC) | Ronzhin 2023 | not wired | P2 |
| ML-driven chunk-size selection | SmartCDC 2025 | not wired, runtime cost too high | P2 |
| Two-tier chunking (sub-chunk dedup) | Wen 2024 | partial — slice map already supports sub-drop ranges | **P1** |

**Decisions:**
- **P1 — Gear+SIMD chunker**: alternative implementation behind a
  `Chunker` trait; profile the existing `FastCDC` first to confirm
  the gap before adopting. Filed as `04-chunker-trait.md`.
- **P1 — Two-tier chunking**: the slice-map already lets a file
  address sub-drop ranges. The win is in *writer-side* dedup at
  sub-chunk granularity for highly-repetitive datasets; revisit
  after dictionary training lands.
- **P2 — LPC-DC, AE-CDC, SmartCDC**: each requires a research
  spike; deferred.

## 2. Deduplication

| Technique | Source | Status | Priority |
|---|---|---|---|
| Per-image content-addressed dedup | design §3 | done | — |
| Shared inline table (small-file dedup) | `WriteContext::build_shared_inline_table` | done | — |
| Cross-image sparse indexing | Lillibridge FAST'09 | not wired | **P1** |
| Extreme binning | Bhagwat 2009 | not wired | P2 |
| Sampling-based dedup | SampleBlock 2023 | not wired | P2 |
| LDMA local-duplicate mining | 2024 | not wired | P2 |

**Decisions:**
- **P1 — Cross-image sparse index**: a sidecar file mapping
  `DropId (truncated) → image_id + slab_ordinal` that the writer
  consults before recompressing. Filed as
  `04-cross-image-sparse-index.md`.
- The other approaches are variations on the same theme; defer
  until we have a real multi-image workload that justifies them.

## 3. Compression

| Technique | Source | Status | Priority |
|---|---|---|---|
| Codec trait + registry (OCP) | this codebase | done | — |
| Tournament selection (per-class) | design §6 | done | — |
| Per-codec tunables (PPMd order+budget, LZMA dict, Bzip2 block) | profile config | **wired in code, ignored in writer** | **P0** |
| ZSTD dictionary training (FastCover) | Collet 2018 | interface stub only | **P1** |
| Multi-stream ZSTD | ZSTD `--ultra -T0` | not wired (we use single-stream `omnizip-zstd`) | P2 |
| PPMd7 (Shkarin DCC'01) | `omnizip-ppmd` 0.13.1 | done at codec layer | — |
| PPMd8 (RESTART+RLE) | `omnizip-ppmd` 0.13.1 | done at codec layer | — |
| Brotli sliding-window per chunk | Brotli RFC | not exposed | P2 |
| Parallel entropy coding (AVX-512) | 2024 wave | omnizip side, not ours | P2 |
| Learned text models (NNCP, etc.) | 2024-2026 | runtime cost too high | P2 |

**Decisions:**
- **P0 — Wire PPMd/LZMA/Bzip2 tunables through the writer**. Today
  `process_file` calls `compress_with_options(codec, chunk, quality)`
  which only honours Brotli quality and ZSTD level; PPMd order/budget
  in `CodecTunables` are silently dropped. Filed as
  `04-ppmd-quality-wiring.md` (despite the name, covers LZMA + Bzip2
  + PPMd7 + PPMd8).
- **P1 — ZSTD dictionary training**. `DictionaryConfig.enabled` is
  a no-op today. Implement a `DictionaryTrainer` trait + a
  `ZstdFastCover` impl behind a `zstd-dict` feature flag. Filed as
  `04-zstd-dictionary-training.md`.

## 4. Indexing & reader path

| Technique | Source | Status | Priority |
|---|---|---|---|
| Merkle B-tree over metadata blob | design §4 | done | — |
| `DropId → slab ordinal` HashMap | `SlabStore::drop_index` | done | — |
| `path → inode` build_path_index | `MetadataBlob::build_path_index` | done | — |
| Memory-mapped slabs (lazy page-in) | `SlabStore::load_mmap` | done | — |
| Streaming decompress (no `Vec`) | `SlabStore::stream_drop` | done | — |
| Hot-slab LRU cache | classic | not wired | **P1** |
| Decoded-drop cache | classic | not wired | **P1** |
| Cuckoo / level hashing for drop index | 2020-2024 | not wired; current HashMap adequate to 10M drops | P2 |

**Decisions:**
- **P1 — Hot slab cache**: a `CachedSlabStore` wrapper with a small
  LRU of recently-decoded drops. Filed as `03-hot-slab-cache.md`.
- HashMap remains the index until profiling shows otherwise.

## 5. I/O & parallelism

| Technique | Source | Status | Priority |
|---|---|---|---|
| `rayon` parallel compression | this codebase | done | — |
| `rayon` parallel extraction | `extract::extract_file` | done | — |
| Memory-mapped reads | `memmap2` | done | — |
| Pipeline parallelism (I/O overlap with compression) | classic producer/consumer | not wired | **P1** |
| `io_uring` batch reads | Linux 5.x | not wired | **P1** |
| `posix_fadvise(WILLNEED)` prefetch hints | POSIX | not wired | **P1** |
| Async/`tokio` integration | ecosystem | deferred — adds heavy dep | P2 |
| GPU-assisted codecs | 2024 wave | runtime cost / driver dep | P2 |

**Decisions:**
- **P1 — Async slab source trait** with a sync mmap impl today;
  io_uring/Linux and kqueue/macOS impls land behind feature flags
  later. Filed as `03-async-slab-source.md`.
- **P1 — Pipeline parallelism**: replace the `par_iter().map(process_file)`
  shape with a producer/consumer pipeline (read I/O on N threads,
  compression on M threads) so we overlap. Filed as
  `04-pipeline-parallelism.md`. **Spec only this cycle** — the
  implementation needs careful benchmarking to confirm a win
  against the current rayon fanout, which is already very fast.
- `tokio` stays out of the dependency graph; the trait is sync.

## 6. RW path

| Technique | Source | Status | Priority |
|---|---|---|---|
| Append-only slabs (write-fast codec) | design §6 | done | — |
| LSM-tree analog (write + turnover) | design §7 | done (`RwImage::commit`/`turnover`) | — |
| Turnover (re-encode, GC unreferenced drops) | design §7 | done | — |
| Tiered compaction (levelled LSM) | RocksDB / 2024 wave | single-tier only today | P2 |
| Write-ahead log for crash safety | classic | **not wired** | **P1** |
| Atomic manifest swap | classic | partial — manifest + slabs written separately | **P1** |
| Copy-on-write B-trees | ZFS / Bcachefs | not wired | P2 |

**Decisions:**
- **P1 — Crash safety**: a WAL that records pending operations
  before commit; recovery on open. Filed as
  `06-rw-crash-safety.md`.
- **P1 — Atomic image swap**: write manifest + slabs to a sidecar
  directory, then `rename(2)` into place. Filed as
  `06-atomic-image-swap.md`.
- Tiered compaction and CoW B-trees need their own research spikes.

## 7. Network / streaming

| Technique | Source | Status | Priority |
|---|---|---|---|
| HTTP range | `08-http-range-streaming.md` | spec only | P1 |
| S3 byte-range | `08-s3-locator.md` | spec only | P1 |
| IPFS CAR | `08-ipfs-car.md` | spec only | P2 |
| BitTorrent-style swarm | 2024-2026 | not wired | P2 |

**Decisions:**
-locator work is already scoped under `08-*`; nothing to add here.

## 8. Integrity

| Technique | Source | Status | Priority |
|---|---|---|---|
| BLAKE3 (SIMD) content hashing | design §3 | done | — |
| Merkle B-tree over sections | `compute_merkle_root` | done | — |
| Parallel Merkle via `blake3::rayon` | workspace dep | done | — |
| Incremental Merkle update for RW | 2024 wave | partial — we re-hash on commit | P2 |
| Signature bundle (sigstore) | `05-signing-sigstore.md` | spec only | P1 |

## 9. Code quality (Ruby rules → Rust spirit)

The user's global Ruby rules translate to Rust as:

| Ruby rule | Rust equivalent | Status | Priority |
|---|---|---|---|
| No `send` to private methods | No `unsafe` to bypass visibility; use the trait system | clean | — |
| No `instance_variable_set/get` | No external `RefCell::borrow_mut` to mutate private state | clean | — |
| No `respond_to?` for type checks | Use trait dispatch / enum, not ad-hoc reflection | clean | — |
| No `require_relative`; use autoload | Module system: declare in immediate parent's `lib.rs` | one-off: verify `mod` declarations in each `lib.rs` | **P0** |
| Model-driven, semantically-driven | Newtypes (`DropId`, `SlabId`, `ManifestRoot`); domain vocabulary | done | — |
| OCP | Codec/categorizer/profile registries | done | — |
| MECE | One owner per concern | done | — |
| DRY | — | `compaction.rs` and `turnover.rs` overlap; `rw.rs` and CLI overlap | **P1** |
| Good specs throughout | TODO.impl/* | this roadmap + per-feature TODOs fill the gaps | **P0** |

**Decisions:**
- **P0 — Module-declaration audit**: every public module is
  declared in its parent's `lib.rs`; no transitive `mod foo` from
  leaf files. Filed inline in this doc as the acceptance criterion.
- **P1 — DRY pass**: extract a shared `LiveTree` walker used by
  `compaction.rs`, `turnover.rs`, `RwImage::commit/turnover`, and
  the CLI's `extract`. Filed as `04-live-tree-walker.md`.

## Acceptance

- [ ] This doc lists every 2026 CDC/dedup/compression/indexing/IO/RW
  technique the project should plausibly adopt, with priority.
- [ ] Each P0/P1 line item has its own TODO file under the right
  phase directory, with status, dependencies, and acceptance.
- [ ] STATUS.md links to this doc.
- [ ] No P2 item is left without an explicit "deferred — reason".

## Cross-references

- 04-ppmd-quality-wiring.md (P0, this cycle)
- 04-zstd-dictionary-training.md (P1)
- 04-pipeline-parallelism.md (P1, spec only)
- 04-cross-image-sparse-index.md (P1)
- 04-live-tree-walker.md (P1)
- 04-chunker-trait.md (P1)
- 03-async-slab-source.md (P1)
- 03-hot-slab-cache.md (P1)
- 06-rw-crash-safety.md (P1)
- 06-atomic-image-swap.md (P1)
