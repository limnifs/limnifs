# Changelog

All notable changes to LimniFS are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.2.63] — 2026-08-25

### Security

- **Locator path-traversal gate (CWE-22).** Every local join site —
  slab loads (`SlabStore`), the external-metadata helper, the RW
  image paths, the FUSE vfs, `FileLocator`, and all CLI reads —
  previously stripped `file:` and joined the raw path against the
  image directory. A malicious manifest could point a slab or
  metadata sidecar at `file:../../etc/passwd` (or an absolute path,
  which `Path::join` substitutes wholesale) and exfiltrate host
  files through `cat`/`extract`. New
  `limnifs_core::locator::local_sidecar_name()` admits only flat
  file names (no separators, no `.`, no `..`, no drive letters) and
  every join site routes through it; the URI grammar itself stays
  permissive per the format spec (rich `file:` paths remain legal
  for future resolver backends — they are simply refused for local
  sidecar access). Writer-emitted locators are always flat, so
  legitimate images are unaffected (verified). End-to-end: a
  manifest tampered to `file:../secretx` now fails with a named
  error on every reader path instead of reading the host file.

## [0.2.62] — 2026-08-25

### Added

- **#190: symlink support in the writer.** The format and reader
  always carried symlinks (`ContentHandle::Symlink`); the walk now
  records them too — relative or absolute targets, in-tree or
  out-of-tree, dangling included. Parent entry typing no longer
  follows links (`entry.file_type()` instead of `entry.metadata()`).
  Anything genuinely unsupported (sockets, FIFOs, devices) now raises
  a named `WriteError::UnsupportedFileType` with guidance instead of
  a bare I/O error. Verified end-to-end on the git-gem shape
  (`.claude/skills -> ../.github/skills`).

### Fixed

- **#191: external metadata has NO reader ceiling — by design, now
  documented and centralized.** `DEFAULT_INLINE_METADATA_MAX_BYTES`
  gates INLINE metadata only (unbounded-manifest-read DoS guard);
  external sidecars are separate files the opener chose to read.
  Verified at 150,000 inodes / 616 MiB sidecar: byte-exact extract.
  New `limnifs_core::read_external_metadata(&reference, &image_path)`
  is the one true load path (limni now uses it); the constant's docs
  say explicitly not to apply it to sidecars.

## [0.2.61] — 2026-08-24

### Changed

- **omnizip 0.16.91 → 0.16.92** — LZMA now tracks system xz -6
  within ~1% on size across the upstream fixture matrix (fits4m
  1.001x, mix2m 1.011x, m329 1.012-1.013x, big100m 1.000x with
  parallel speedup; -9 within 4 B). xz/zstd conformance canaries
  still byte-exact against the system CLIs; 662 tests green.

## [0.2.60] — 2026-08-24

### Changed

- **omnizip 0.16.89 → 0.16.91** — routine absorption. xz/zstd
  system-CLI conformance canaries still byte-exact; 662 tests green.
  Quick benchmark: ratios unchanged (CSV 5.71%, FITS 32.09%, WAV
  0.02%); FITS create improved to 4.0 s (was 6.9 s).

## [0.2.59] — 2026-08-23

### Changed

- **omnizip 0.16.88 → 0.16.89** — upstream fixed omnizip#329 (PR
  #330): mid-stream raw chunks now carry control 0x02 (dictionary
  preserved) and any dictionary-resetting chunk forces a
  props-carrying level-2 LZMA chunk after it. Validated against the
  issue's 11-case trigger matrix (11/11 `xz -t` clean) and the
  100 MB mixed fixture: system xz decodes rc=0, byte-exact, full
  length — XZ output is now fully conformant for external interop.
  Ratio unchanged (conformance fix, as upstream noted).

## [0.2.58] — 2026-08-23

### Changed

- **omnizip 0.16.80 → 0.16.88** — absorbs PR #327 (four LZMA fixes:
  chunk bisection past the u16 size field + raw-store chunks,
  LZMA2 without the LZMA1 end-of-payload marker, conditional range-
  coder tail byte, two xz container field corrections) and PR #328
  (zstd minMatch floor + RLE literals writer + hardened weight
  fallback; Best no longer corrupts above ~1 MiB). Validated: 100 MB
  mixed payload round-trips through both codecs; the zstd frame is
  accepted byte-exact by the system CLI.
- **Known upstream gap (omnizip#329, filed with a trigger matrix and
  repro):** XZ frames that exercise the new bisect/raw-store path
  (>=2 MiB total with >=~512 KiB incompressible content) are
  non-conformant — the reference xz decodes a byte-exact prefix,
  stops N bytes short, then errors. Self-readable; affects external
  tool interop only.

## [0.2.57] — 2026-08-23

### Added

- **#189: `shared_inline` write knob** — `WriteConfig.defaults.shared_inline`
  (default `true`, TOML `[defaults] shared_inline = false`). Setting
  it `false` skips the shared-inline dedup pass and emits plain
  `INLINE_DATA` inodes, so images containing duplicate small files
  stay readable by pre-#186 readers (published tebako runtimes whose
  reserved mask rejects the `SHARED_INLINE` flag) at the cost of
  duplicated inline bytes. Fully additive; default behavior
  unchanged. Regression test covers the off path.

## [0.2.56] — 2026-08-23

### Changed

- **omnizip 0.16.79 → 0.16.80** — routine absorption. Quick
  benchmark stable (CSV 1.0 s / 5.71%, FITS 6.9 s / 32.09%, WAV
  0.18 s / 0.02%, tiny-files 6.2 s); win count over DwarFS 9-0.

## [0.2.55] — 2026-08-22

### Changed

- **omnizip 0.16.78 → 0.16.79** — upstream fixed the residual
  omnizip#315 decoder defect (PR #317): the decoder now correctly
  reads frames its own encoder produces at every level. Verified
  against both the original 318-byte repro and the 163-byte minimal
  case we shrank for the upstream report — all five levels
  round-trip.
- **zstd write-side self-check REMOVED** — the v0.2.53 decompress-
  verify guard is obsolete; encode paths are direct again (drops the
  per-drop decode overhead). The #315 repro stays pinned as a
  canary test that now requires round-trip at all levels; if it ever
  regresses, the guard documented in v0.2.53 comes back.
- Routine dependency updates (cc, either, log, ref-cast).

## [0.2.54] — 2026-08-22

### Changed

- **omnizip 0.16.77 → 0.16.78** — absorbs the zstd offset-code
  table fix (PR #316): real `zstd` CLI frames now decode correctly.
  Retested against the omnizip#315 repro: the 318-byte blob still
  fails its self-round-trip identically at
  Fastest/Fast/Default/Better (Best OK), so the v0.2.53 write-side
  zstd self-check remains in place; retest results posted upstream.

## [0.2.53] — 2026-08-22

### Fixed

- **#186: shared-inline images unreadable** —
  `INODE_FLAG_RESERVED_MASK` (0xF8) covered the defined
  `INODE_FLAG_SHARED_INLINE` (0x08), so the reader rejected every
  deduplicated shared-inline inode the writer emits. Mask corrected
  to 0xF0; round-trip conformance test added (found by tebako's
  LimniFS integration).
- **#187: metadata externalized below the reader ceiling** — the
  writer externalized the metadata sidecar above 768 KiB while every
  default reader accepts 1 MiB inline. The default threshold is now
  derived from the reader ceiling (1 MiB − 24 KiB headroom), is
  configurable via `WriteConfig.defaults.metadata_externalize_threshold`,
  and is clamped at the reader ceiling in `assemble` so no
  configuration can emit unreadable inline metadata.
- **#188 / omnizip#315 mitigation: zstd self-check on write** —
  omnizip-zstd's decoder (still broken in 0.16.77, content-dependent)
  fails on frames its own encoder produced at
  Fastest/Fast/Default/Better; Best is correct. Every LimniFS zstd
  encode now decompress-verifies its own frame and refuses to emit
  ones that do not round-trip — the tournament moves to the next
  codec instead of writing an image no reader can open. Remove when
  the upstream decoder is fixed. Regression test uses the exact
  318-byte blob from the upstream issue.
## [0.2.52] — 2026-08-22

### Changed

- **omnizip 0.16.76 → 0.16.77** — the reference-parity campaign
  landed: the from-spec Brotli dictionary-lookup cost is fixed
  upstream. Measured locally: 51 MB metadata-blob compress drops
  24.3 s → 6.7 s (3.6x). Quick benchmark (balanced): WAV create
  12.2 s → 0.16 s at 0.02% ratio; FITS 14.5 s → 5.7 s; CSV create
  2.25 s → 0.82 s. Note the CSV ratio trade: 2.99% → 5.71% (still
  ahead of the 6.23% C reference; DwarFS's LZMA reaches 3.59% but
  takes 73x longer to create).

## [0.2.51] — 2026-08-21

### Added

- **Streaming directory walk** (TODO.perf/15) — `write_directory_with_config`
  overlaps the tree walk with compression: bounded mpsc producer →
  rayon `par_bridge` consumers, re-sequenced to walk order. Output
  bytes identical (verified on 50K-file / 400 MB tree: manifest,
  sidecar, all 7 slabs); ~10% faster warm-cache deep-tree create,
  more on cold cache.
- **assemble phase tracing** — `LIMNIFS_TRACE_ASSEMBLE=1` prints
  per-phase timings (pack_slabs / shared_inline_table /
  metadata_encode / metadata_compress).

### Fixed

- **`inline_threshold` config is honored** — the knob was silently
  ignored (hard constant); now threaded through WriteContext into
  the walk + skip-chunking decisions. Per-profile values apply.
- **`cat-multi` drop cache** — routes through `CachedSlabStore` like
  `extract`; files sharing drops decode once (400 files / 100 drops:
  0.30 s).
- FSST+Brotli CSV round-trip test un-ignored (omnizip 0.16.64+
  fixed dictionary-reference classification).

### Changed

- **omnizip 0.16.75 → 0.16.76.** First full benchmark on the 0.16.7x
  line: CSV create 88 s → 2.25 s (39x) at 2.99% ratio (vs reference
  6.23%), FITS 276 s → 14.5 s, tiny-files 5.8 s.
- TODO.perf board closed: 01/04/08 rejected with data (Brotli keeps
  metadata on ratio; inline routing beats slabs on tiny files;
  categorizer dispatch <1%), 02/05 verified shipped.

### Changed

- **Release process aligned with omnizip-rs / parsanol-rs** —
  release-plz now owns crates.io publishing: version bumps land on
  main as commits, release-plz tags each crate `<crate>-vX.Y.Z` and
  publishes in DAG order via trusted publishing (OIDC; no static
  registry token). Internal deps moved to `workspace.dependencies`
  (`limnifs-core.workspace = true` style). Binary/SBOM workflows
  unchanged, now also keyed off release-plz's `limni-v*` tags.

## [0.2.50] — 2026-08-20

### Fixed

- **aarch64-musl static build**: musl.cc tarballs truncate on GH
  runners; build that target with `cross` (containerized full musl
  toolchain) instead.

## [0.2.49] — 2026-08-20

### Fixed

- **Release matrix (gnu)**: set
  `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc`
  — host `cc` was linking x86-64 flavor lld against aarch64 objects.
- **Release binaries (musl)**: install `musl-tools` (x86_64) and the
  musl.cc prebuilt aarch64 cross toolchain; blake3's NEON shim needs
  a target C compiler.
- **Windows binary**: build with host MSVC (no mingw on the runner).

## [0.2.48] — 2026-08-20

### Fixed

- **Release build matrix**: `aarch64-unknown-linux-gnu` installs
  `gcc-aarch64-linux-gnu` — blake3's NEON shim needs a cross C
  compiler; the target previously failed with
  `ToolNotFound: aarch64-linux-gnu-gcc`.

## [0.2.47] — 2026-08-20

### Fixed

- **no-shims**: `todo!` annotations must be **same-line** (previous
  attempt put the task path on the next line; gate still failed).
- **clippy 1.97**: `dict_round_trip` map_unwrap_or + cast.
- **SBOM**: `cargo-cyclonedx` 0.5 dropped `--workspace`; use
  `--manifest-path ./Cargo.toml -f json`.
- **rustdoc Pages**: use default `github-pages` artifact name.
- **GitHub Release upload**: flatten SBOM paths; do not fail the
  whole release on a missing optional glob.

## [0.2.46] — 2026-08-20

### Fixed

- **CI unblocked end-to-end** — no-shims gate annotations for
  `todo!`/`#[ignore]`, clippy 1.97 pedantic on the FLAC corpus test,
  WASM release skips when `limnifs-wasm` is absent, rustfmt clean.
- **Internal path-dep versions** — crates pinned each other at
  `0.2.0` while the workspace was at `0.2.4x`. Bumped to match so
  crates.io publish resolves.

### Added

- **GHA crates.io publish** — `release.yml` now publishes
  `limnifs-format` → `limnifs-ocb3` → `limnifs-core` →
  `limnifs-write` → `limni` on `v*` tags when
  `CARGO_REGISTRY_TOKEN` is set. Releases are GHA-only; local
  `cargo publish` is not the release path.
- `publish = false` on `limnifs-bench` and `limnifs-conformance`.

### Note

Prior "releases" v0.2.20–v0.2.44 were Cargo.toml version bumps on
`main` without tags and without crates.io publish. Downstream that
needs a pin should use git tags from v0.2.46 forward, or a git rev
until the crates.io publish job succeeds.

## [0.2.45] — 2026-08-20

### Fixed

- **CI: `pipeline-parallelism` feature builds again** — `pipeline.rs`
  still used `Vec<u8>` for compressed bytes and returned bare
  `WriteArtifact` from a `Result`-returning function after the
  v0.2.42 `Arc<[u8]>` change. Now matches `RawDrop` and wraps
  `assemble()` in `Ok(...)`. Unblocked `--all-features` CI gates.
- **CI: E2E workflow CLI args** — `limni limn` takes positional
  `SOURCE OUTPUT`, not `-o`. Workflow was copied with the wrong
  flag and failed before any real E2E ran.
- **CI: rustdoc workflow** — de-confium'd path globs and index
  generation so it targets `limni` / `limnifs-*` crates.
- **CI: clippy `-D warnings`** — `manual_let_else` in the FLAC
  corpus test and a `doc_markdown` miss on `SlabStore` in
  `dict_round_trip`.
- **rustfmt** — `cargo fmt --all` so the shared rust-ci fmt check
  is clean.

## [0.2.44] — 2026-08-20

### Changed

- **blake3 1.5 → 1.8.7** — pin the floor to the first blake3 release
  that dropped the `arrayref` dependency. Prior caret `blake3 = "1.5"`
  admitted 1.8.7 on fresh index views, but a stale crates.io CDN edge
  could still resolve onto blake3 1.5.0 → `arrayref ^0.3.5` (all
  yanked) and kill the build. Flooring at 1.8.7 makes the resolution
  fail-closed on those edges instead of resolving onto a dead dep
  graph. `arrayref` is gone from `Cargo.lock`.
- **omnizip 0.16.64 → 0.16.75** — absorb the latest pure-Rust codec
  line. Workspace tests green end-to-end.

## [0.2.43] — 2026-08-19

### Changed

- **omnizip 0.16.62 → 0.16.64** — upstream fixed the from-spec
  Brotli encoder's dictionary-reference classification (walks used
  the unclamped output position; the decoder clamps to
  min(pos, MAX_BACKWARD_DISTANCE)), which caused both the >=16 MB
  tail-window panic and a silent wrong-word corruption variant at
  metablock offsets past the 16 MiB window. Verified against our
  minimal repro (16,976,720-byte FITS-like input): q5 round-trips
  byte-identical where 0.16.62/0.16.63 panicked. The v0.2.42 panic
  guard remains as defense-in-depth.
- Benchmark on 0.16.64: CSV 4.04% (matches upstream's number),
  FITS 32.09% (ricepp keeps the drop; Brotli at 64% is uncompetitive
  on gradient data), WAV 0.44%. CSV/FITS create times increased
  (88s/276s) — the Brotli trial now completes instead of panicking
  early; the dictionary-lookup O(N x dict_words) cost (see
  BUGREPORT-brotli-dict-lookup-O(n).md upstream) is the remaining
  wall-clock factor.

## [0.2.42] — 2026-08-19

### Fixed

- **Codec panics no longer kill the writer** — the codec registry's
  dispatch (compress, compress_with_tunables, decompress) converts
  codec panics into `Err(Corrupt)` via `catch_unwind`, and
  `process_whole_file_drop` falls back Brotli -> ZSTD -> STORE instead
  of propagating the error. Triggered by omnizip-brotli 0.16.62's
  from-spec encoder, which panics past its final partial 2 MiB window
  on ~16 MB+ inputs (BUGREPORT-brotli-tail-window-oob.md; FITS-like
  data minimal repro: 16,976,720 bytes). Metadata-blob Brotli and
  the FSST+Brotli composite internals route through the same guard.
- **Slab magic errors say "slab"** — `parse_slab_header` raised
  `BadMagic` whose message hardcodes "manifest"/"LMFS"; it now
  reports `bad slab magic: expected LIM1`.

### Added

- **Offline Ed25519 sign-then-verify CLI workflow** (TODO.perf/25) —
  `limni sign-keygen` (OpenSSL-compatible PKCS#8/SPKI PEM, verified
  interop both directions), `limni limn --sign-key` signs the image's
  `ManifestRoot` and writes a canonical `<image>.limsig` sidecar,
  `limni verify-sig --pubkey` recomputes the root and checks
  signature + signer identity against the trusted key (three
  independent tamper checks, fully offline), `limni extract
  --verify-key` gates extraction on the same check. Dependency-free
  fixed-layout PEM/PKCS#8 codec in `limnifs-core::signing`.

### Changed

- **`Arc<[u8]>` for compressed drop bytes** (TODO.perf/22) —
  `PendingDrop::compressed` and the cross-file compress cache share
  bytes via Arc; cache hits are refcount bumps instead of deep Vec
  copies. Output bytes unchanged (verified byte-identical on a
  dedup-heavy 200-file tree).
- `limni slab` prints real codec names for all registered codecs
  via `codec_name()` instead of `??` beyond store/lz4.
- Removed orphaned doc comments describing removed cosign shell-out
  commands.

### LimniFS state at v0.2.42

- omnizip 0.16.62 across the board (CSV ratio back to 4.0%)
- Quick-benchmark stable end-to-end: FITS 32.1% (ricepp), CSV 4.0%,
  WAV 0.44%, zeros 0.01%; every verify op is a LimniFS win

## [0.2.39] — 2026-08-06

### Changed

- **Dependency updates** — blake3 1.8.5 → 1.8.6, clap 4.6.5 → 4.6.6,
  zerocopy 0.8.55 → 0.8.56. All semver-compatible patches; no API
  changes. omnizip remains at 0.14.20 (latest published).

### LimniFS state at v0.2.39

- 586 workspace tests passing
- 19 codec variants (LZ4, LZ4-HC, ZSTD, XZ/LZMA, Brotli, DEFLATE,
  libdeflate, Snappy, FLAC, ricepp, FSST+Brotli, BLOSC+Shuffle+LZ4,
  ZPAQ, PPMd7, PPMd8, GLZA, Shuffle+ZSTD, Bitshuffle+LZ4, BZip2,
  Deflate64, BCJ composites ×4)
- 9 profiles (read-only + read-write)
- `write_layer` overlay API for container-image pattern
- `write_stream` streaming API for pipe-from-reader workloads
- Cross-file compress cache for dedup workloads
- mmap input for large files, madvise prefetch on slab reads
- Tournament short-circuit + whole-file short-circuit
- FastCDC 4× unrolled gear hash, parallel slab assembly
- 16 of 17 omnizip crates pure-Rust (only `brotli` crate remains)
- `limni inspect`, `limni verify`, `limni extract`, `limni mount`,
  `limni limn`, `limni cat`, `limni ls`, `limni stat`
- `limni open/commit/turnover/add/update/delete` for RW images

## [0.2.38] — 2026-08-06

### Changed

- **Drop-record batch pre-allocation** — `encode_slab`'s `drop_records`
  Vec now pre-allocates `drops.len() * 49` capacity. Each drop record
  is a fixed 49-byte entry; pre-sizing avoids per-drop realloc.

## [0.2.37] — 2026-08-06

### Added

- **`write_layer` API** — the headline feature closing the
  overlay/ComposeFS gap. New entry point
  `limnifs_write::write_layer(base_image, root, config)` produces a
  `.lim` image that **references** a base image's drops rather than
  re-encoding them. Chunks whose `DropId` exists in the base are
  recorded only as `PendingSlice` references — no slab bytes are
  emitted in the layer. The reader resolves them via the overlay
  chain at read time.

  The resulting manifest carries a `delta_linkage` section with the
  base's `ManifestRoot`, so any reader that supports overlay chains
  can extract the layer standalone (if all drops are local) or
  stacked on the base.

  Closes TODO.perf/23. This is the container-image pattern: a 1 GB
  base + 10 MB layer is now built, stored, and distributed as two
  independent images with near-zero overhead on the reused content.

### Internal

- **`CODEC_REFERENCED` (0xFE)** — sentinel codec id for drops that
  are resolved via the overlay chain. Never appears in slab drop
  records; used only as an in-memory marker in `PendingDrop::codec`
  so `pack_slabs` knows to skip them.
- **`SlabStore::drop_index_keys()`** — new public accessor that
  yields an iterator over every `DropId` known to the store. Used by
  `load_base_drop_index` to build the base's drop set for layer
  dedup.
- **`write_directory_body`** — shared body extracted from
  `write_directory_with_config`. Both the standalone and layer entry
  points now use it. DRY: no parallel-iteration code duplication.
- **New TODO.perf files** — 15 through 25 covering the remaining
  LimniFS-side perf + feature work. TODO.perf/23 (write_layer) is
  done in this release; the rest are planned.

## [0.2.36] — 2026-08-05

### Added

- **Cross-file compress cache** — each rayon worker thread now carries
  a thread-local `HashMap<DropId, (codec_id, compressed_bytes)>`. When
  two files share a chunk (common in source trees, container layers,
  build artifacts, and tiny-files benchmarks), the second file hits
  the cache and skips the tournament compress pass entirely.

  Cache is bounded at 100K entries (~16 GB worst case at 16 KB/chunk);
  eviction is "stop inserting once full" for simplicity. Output bytes
  are unchanged — the cache reuses the same `(codec_id, compressed)`
  the tournament would have produced.

  Most impactful on workloads with significant duplicate content.
  Tiny-files benchmark with 50K identical 18-byte files improved
  modestly (~7%); the dominant cost there is per-file I/O and
  categorization, not compression. Real workloads (layered container
  images, successive build outputs, source trees with vendored deps)
  will see larger gains proportional to their redundancy.

## [0.2.35] — 2026-08-05

### Changed

- **omnizip 0.14.20** — bumped all 17 omnizip-* crates from 0.14.19
  to 0.14.20. Brings omnizip-rs TODO 116 — **libdeflate dynamic-Huffman
  correctness fix**: the broken package-merge was replaced with
  correct standard Huffman + zlib CPI length limiting. Dynamic-Huffman
  path re-enabled.

  **`CODEC_LIBDEFLATE` (0x14) is the LimniFS codec most affected.**
  Text and binary inputs see 10-20% better ratio (previously the
  dynamic-Huffman path was degrading to stored blocks). Our
  libdeflate wrapper automatically benefits — no code change
  required.

  Round-trip safety verified by the existing `cross_decodes_with_deflate`
  test, which exercises the wire format with the miniz_oxide-backed
  `CODEC_DEFLATE` (0x05) decoder.

## [0.2.34] — 2026-08-05

### Changed

- **omnizip 0.14.19** — bumped all 17 omnizip-* crates from
  0.14.17/0.14.18 to 0.14.19. Headline: **omnizip-lz4 from-spec
  encoder bug fixed** (omnizip-rs PR #115 — the LimniFS-discovered
  extension-byte bug at the LZ4 code-nibble-15 boundary).

  The Cargo.lock pin to omnizip-lz4 0.14.17 (workaround from v0.2.33)
  is **removed**. The from-spec encoder is now the only LZ4
  implementation in the dep tree.

### Removed

- **lz4_flex transitive dependency eliminated.** omnizip-blosc and
  omnizip-filters previously pulled lz4_flex for their LZ4 paths;
  both now use the in-house omnizip-lz4 from-spec encoder. LimniFS's
  dep tree is one step closer to fully first-party.

  Only one external codec dep remains anywhere in the omnizip
  stack: `brotli` (used by omnizip-brotli, awaiting Phase C of the
  full in-house port — omnizip TODO 151).

## [0.2.33] — 2026-08-05

### Changed

- **omnizip 0.14.18** — bumped 16 of 17 omnizip-* crates from 0.14.17
  to 0.14.18. **`omnizip-lz4` is pinned to 0.14.17** because the new
  from-spec LZ4 encoder (omnizip-rs TODO 132) produces output its own
  decoder rejects on inputs above ~64 bytes. See
  `docs/omnizip-proposals/lz4-from-spec-broken.md` for the bug report
  and acceptance criteria.

  Other 0.14.18 changes are absorbed cleanly: omnizip-lzma's
  `ResetMode::Warm` (third reset tier beyond `Full` and `ReuseState`),
  continued Brotli Phase C work, and ongoing CI/determinism hardening.

### Workaround

`Cargo.lock` pins `omnizip-lz4` to 0.14.17 (which uses the
`lz4_flex` wrap, works correctly). All other omnizip-* crates are at
0.14.18. When omnizip ships the LZ4 from-spec encoder fix, remove the
pin via `cargo update -p omnizip-lz4`.

## [0.2.32] — 2026-08-05

### Changed

- **omnizip 0.14.17** — bumped all 17 omnizip-* crates from 0.14.16 to
  0.14.17. Brings the LZMA `ResetMode` API (omnizip-rs TODO 165
  closed). `LzmaCompressor::with_reset_mode(ResetMode::ReuseState)`
  carries probability-model adaptation across compress calls, skipping
  the per-call state reset. ~5–10% encode speedup on `max-ratio` batch
  workloads where output determinism across runs doesn't matter.

### Added

- **`LIMNIFS_XZ_REUSE_STATE` env var** — opt into `ResetMode::ReuseState`
  for the thread-local `LzmaCompressor` in `limnifs-core::codec::xz`.
  Default behaviour (deterministic `ResetMode::Full`) is unchanged;
  users who don't care about run-to-run byte-determinism can set the
  env var for faster LZMA encode in the `max-ratio` profile tournament.

  Round-trip safety is unaffected — each compressed blob carries its
  own LZMA2 chunk-header reset markers. What changes is that the same
  input compressed in different runs may produce different bytes
  (state inheritance depends on prior calls on the same thread).

## [0.2.31] — 2026-08-05

### Changed

- **omnizip 0.14.16** — bumped all 17 omnizip-* crates from 0.14.14 to
  0.14.16. Three upstream releases absorbed:

  - **0.14.15: Snappy from-spec encoder + LZMA match-finder reuse**.
    `CODEC_SNAPPY` (0x06) was decode-only in earlier LimniFS releases;
    the existing tests asserted round-trip but the encoder was a stub
    that produced near-incompressible output. Now the from-spec port
    produces real Snappy streams (verified by the existing
    `snappy_compresses_repetitive_data` test which asserts compressed
    output is smaller than input on highly repetitive data).
  - **0.14.16: Snappy snap-compat (full wire-format compatibility)**.
    Snappy output is now byte-compatible with Google's `snappy` CLI
    and the upstream C++ reference.
  - **LZMA match-finder reuse** — `LzmaCompressor` (which our
    `XzCodec` already uses via thread-local since v0.2.26) now
    amortizes match-finder state across calls. Real encoder-state
    reuse lands automatically; no LimniFS-side change required.

### Effect on profiles

`CODEC_SNAPPY` is registered but not in any default profile's
tournament — its primary use case is round-trip with externally
produced Snappy streams (Parquet, ORC, Avro, SQLite WAL). Users who
want Snappy output can add it to a custom profile's tournament list.

The LZMA match-finder reuse benefits the `max-ratio` profile (where
XZ/LZMA is in the tournament). Single-iteration benchmark shows
run-to-run noise dominates the measurement; medians over 3+ runs
recommended for any regression check.

## [0.2.30] — 2026-08-05

### Added

- **`write_stream` API** — new public entry point in `limnifs-write`
  packs a single named stream from any `std::io::Read` into a `.lim`
  image without buffering the full content. Uses `FastCDC::chunk_reader`
  internally; internal buffering is bounded at `max_chunk_size + 64 KiB`.
  Callers piping from a network socket, pipe, or generator no longer
  need a temp file.

  ```rust
  let reader = std::io::Cursor::new(my_bytes);
  let artifact = limnifs_write::write_stream("output.txt", reader, &config)?;
  ```

- **Memory-mapped input** — `process_file` now memmaps files above
  1 MiB instead of `std::fs::read`-ing them into a `Vec`. Pages load
  on demand from the kernel page cache. For multi-GiB inputs, peak
  RSS drops from `total_input_size` to roughly `unique_chunks ×
  avg_chunk_size` because chunk compressors see borrowed slices into
  the mmap. Below 1 MiB the crossover favours plain reads.

### Changed

- **FSST+Brotli accepts pre-computed Brotli baseline** — new public
  function `limnifs_core::codec::fsst_brotli::compress_with_baseline`
  takes `Option<&[u8]>` for an already-computed Brotli baseline.
  `process_whole_file_drop` passes its baseline when the categorizer
  routes to FSST+Brotli so the codec doesn't re-compress the plaintext
  with Brotli just for comparison. Eliminates one full Brotli pass
  per FSST-routed file.

### Tests

- New unit test `write_stream_packs_single_named_stream` covers the
  streaming API end-to-end.
- Full workspace test suite: 585/585 passing (was 584).

## [0.2.29] — 2026-08-05

### Changed

- **omnizip 0.14.14** — bumped all 17 omnizip-* crates from 0.14.12 to
  0.14.14. Headline upstream change: **omnizip-rs TODO 152 closed —
  ZSTD Huffman encode table cache, 7.5× ZSTD encode speedup**. Also
  ships Brotli Phase C (TODO 117 further along) and continued
  FLAC/ricepp improvements.

### Benchmark impact (synthetic, balanced profile)

| Dataset | 0.14.12 | 0.14.14 | Speedup |
|---|---:|---:|---:|
| fits-synthetic | 26.94 s | 3.68 s | **7.3×** |
| tiny-files (max-ratio) | 1.93 s | 1.07 s | 1.8× |
| csv-synthetic (max-ratio) | 33.12 s | 29.72 s | 1.1× |
| fits-synthetic (max-ratio) | 189.45 s | 97.75 s | 1.9× |

ZSTD is in 4 of 9 LimniFS profiles (max-read, max-read-rw, balanced-rw
text path, max-ratio tournament). The 7.5× ZSTD encode speedup moves
benchmark numbers across many datasets — FITS sees the biggest gain
because its 47 MB payload runs through ZSTD L6 baseline.

## [0.2.28] — 2026-08-05

### Changed

- **`process_whole_file_drop` short-circuit** — when Brotli already
  achieves < 5% ratio on a categorizer-routed file (CSV, WAV, etc.),
  skip the ZSTD pass. The file is highly compressible text/audio and
  ZSTD is unlikely to beat Brotli by enough to justify the extra
  pass.

  Measured impact on synthetic datasets (balanced profile):

  | Dataset | Before | After | Speedup |
  |---|---:|---:|---:|
  | csv-synthetic | 2.95 s | 0.37 s | **8×** |
  | wav-synthetic | 1.40 s | 0.13 s | **11×** |

  Same output bytes, same ratios (3.6% CSV, 0.0% WAV) — pure speedup
  from removing redundant ZSTD work that wouldn't have won anyway.

  FITS, random, tiny-files, repetitive, zeros all unchanged (within
  noise) — their Brotli ratios are > 5% so the short-circuit doesn't
  fire.

## [0.2.27] — 2026-08-05

### Changed

- **omnizip 0.14.12** — bumped all 17 omnizip-* crates from 0.14.11
  to 0.14.12. Brings:
  - **FLAC FFT autocorrelation** (omnizip-rs TODO 112 — `fft-acf`
    feature, O(N log N) vs O(N·order)). Half of the 10× FLAC LPC
    speed gap closed.
  - **ricepp SIMD delta** (omnizip-rs TODO 113 — `simd-delta`
    feature with `wide::u64x4`). Half of the 6× ricepp speed gap
    closed.

  Updated `docs/omnizip-proposals/flac-lpc-finish.md` and
  `docs/omnizip-proposals/ricepp-simd-delta.md` with the new
  status. FLAC categorizer remains off-by-default pending
  LimniFS-side corpus verification; ricepp speedup is automatic
  (LimniFS doesn't own the encoder).

  Measured impact on synthetic `fits-synthetic` (47 MB,
  balanced profile): create time 25.2 s → 23.6 s (~6% faster). The
  remaining gap vs ZSTD is the encoding decision itself (ricepp
  is a specialised integer-pixel codec; ZSTD has a richer
  general-purpose model).

## [0.2.26] — 2026-08-05

### Added

- **`CODEC_LIBDEFLATE` (0x14)** — second independent DEFLATE
  implementation alongside `CODEC_DEFLATE` (0x05). Both are RFC 1951
  DEFLATE wrapped in RFC 1950 zlib; `0x14` uses `omnizip-libdeflate`
  (omnizip's pure-Rust port — LZ77 + fixed-Huffman encode, canonical
  Huffman inflate, optimized for decode speed), `0x05` uses
  `omnizip-deflate` (wraps `miniz_oxide`). Wire-compatible: a writer
  using `0x14` produces output decodable by a reader using `0x05`
  and vice versa.

  **Upstream Adler-32 bug worked around.** `omnizip-libdeflate` 0.14.6
  computes the zlib trailer's Adler-32 over the compressed stream
  instead of the plaintext (RFC 1950 §9 violation). miniz_oxide and
  `gzip -d` reject these streams. Our wrapper re-computes the Adler-32
  over the plaintext and patches the trailer before returning, so
  output is byte-compatible with `gzip`/`zlib`/0x05. Bug report and
  acceptance criteria: `docs/omnizip-proposals/libdeflate-adler32.md`.

  5 new unit tests: round-trip text, round-trip empty, cross-decode
  with `DeflateCodec` (both directions), length-mismatch rejection,
  Adler-32 known values.

### Changed

- **`XzCodec` now routes through `LzmaCompressor`** — uses omnizip-lzma
  0.14.11's reusable-state API (per-rayon-worker thread-local
  `LzmaCompressor`) instead of one-shot `xz_compress`. Today's
  benefit is mainly forward compatibility — when omnizip adds real
  per-call encoder-state reuse (TODO 146 follow-ons), our wrapper
  picks it up automatically. Adds `compress_with_tunables` and
  `PerCodecTunables` impls so the writer can pass level/lc/lp/pb
  via `CodecTunables::quality`.

## [0.2.25] — 2026-08-05

### Changed

- **omnizip 0.14.11** — bumped omnizip-lzma and omnizip-zstd from
  0.14.10 to 0.14.11. Brings omnizip-rs PR TODO 136 (libdeflate
  pure-Rust — `omnizip-libdeflate` no longer carries a `miniz_oxide`
  fallback in its `[dependencies]`) and TODO 146 (reusable-state
  sweep — `LzmaCompressor` mirrors `ZstdCompressor`/`PpmdCompressor`).

- **First-party codec migration** — per the project rule "prefer
  omnizip-* over third-party codec crates", the codec wrappers in
  `limnifs-core/src/codec/` now route through omnizip APIs end-to-end:

  | Wrapper | Before | After |
  |---|---|---|
  | `lz4.rs` (LZ4 fast + HC) | `lz4_flex` direct | `omnizip_lz4::{Lz4FastCodec, Lz4HcCodec}` |
  | `deflate.rs` | `miniz_oxide` direct | `omnizip_deflate::DeflateCodec` |
  | `brotli.rs` | `brotli` direct | `omnizip_brotli::BrotliCodec` |
  | `zstd.rs` | already omnizip | unchanged |

  Removed direct deps: `brotli`, `lz4_flex`, `miniz_oxide`, `ruzstd`.
  These still appear transitively (omnizip-brotli wraps brotli;
  omnizip-deflate and omnizip-libdeflate use miniz_oxide; omnizip-lz4
  uses lz4_flex), but LimniFS no longer imports them directly — the
  codec stack is omnizip end-to-end.

  Wire format unchanged: same codec ids, same bytes, same round-trip
  behavior. All 579 workspace tests pass.

## [0.2.24] — 2026-08-05

### Changed

- **FastCDC 4× unrolled inner loops** — `find_boundary` now processes
  4 bytes per iteration in both the mask1 and mask2 phases. The
  four gear lookups per iteration can be hoisted into a vector load
  by the optimiser; the four mask checks fold into a vector
  compare. Sequential shift-and-add is unchanged — the gear hash's
  loop-carried dependency means true SIMD requires either
  `pclmulqdq` (nightly-only `std::simd`) or a leap-based CDC
  rewrite (changes wire-format boundaries). See
  `docs/fastcdc-simd-proposal.md` for the full algorithmic
  analysis. Closes TODO.perf/06.

- **Parallel slab encoding** — `pack_slabs` now has two phases:
  sequential slab grouping (per-drop size budget — must be
  sequential), then parallel `encode_slab` across rayon workers.
  Cross-slab parallelism gives N-core speedup on multi-slab images.
  Within a slab, drop records + solid window stay sequential (per-
  slab size is bounded; cross-slab was the bigger win). Closes
  TODO.perf/07.

## [0.2.23] — 2026-08-05

### Changed

- **omnizip 0.14.10** — all 17 omnizip-* crates bumped from 0.14.8 to
  0.14.10. omnizip-rs PR #90 fixes the ZSTD Default/L6+ regression
  on highly-repetitive inputs that LimniFS 0.2.21 had to work around.

### Restored

- **ZSTD full level mapping** — `level_for_quality` in
  `limnifs-core/src/codec/zstd.rs` no longer caps at `Fast` (L3).
  The original `6..=11 → Default, 12..=21 → Better, 22+ → Best`
  mapping is restored. Profiles that requested `quality: 11` (max-ratio,
  max-read) now actually get ZSTD L6 as intended.
- **L1-vs-L6 regression test** —
  `zstd_higher_levels_compress_better_than_lower` compares Default
  (L6) vs Fastest (L1) again. The L1-vs-L3 variant that got us
  through the 0.14.8/0.14.9 regression window is reverted.

  See `docs/omnizip-proposals/zstd-default-broken.md` for the full
  incident timeline.

## [0.2.22] — 2026-08-05

### Added

- **Tournament short-circuit** — `process_file` now iterates
  `WriteConfig::tournament.codecs` for every chunk above
  `min_size_threshold` and accepts the first codec that reaches
  `tournament.short_circuit_threshold` (per-mille, default 250 =
  25% of original size). On compressible text chunks, LZ4 typically
  reaches <10% ratio in microseconds — the short-circuit accepts
  that and skips the much-slower Brotli pass we would otherwise run.

  Per-profile thresholds (per-mille):

  | Profile | Threshold | Behaviour |
  |---|---:|---|
  | `max-ratio` | 0 | Try every codec, pick smallest |
  | `max-speed` / `max-write` / `max-write-rw` | 500 | Accept almost any codec |
  | `balanced` / `competitive` / `balanced-rw` | 250 | Accept first codec < 25% |
  | `max-read` / `max-read-rw` | 200 | Tighter — favor ratio |

  Binary chunks with `skip_for_binary` and chunks below
  `min_size_threshold` bypass the tournament entirely (matches
  v0.1 behaviour). Categorizer-routed files (FLAC, RICEPP,
  FSST+Brotli) still go through `process_whole_file_drop` and are
  not affected.

  Closes TODO.perf/14.

## [0.2.21] — 2026-08-05

### Changed

- **omnizip 0.14.8** — all 17 omnizip-* crates bumped from 0.14.6 to
  0.14.8. Brings the ZSTD backward-extension infinite-loop fix
  (omnizip-rs PR #85) and the FLAC LPC + ricepp speed improvements
  (omnizip-rs PR #84). The stale `omnizip-flac = "0.10"` entry in
  `limnifs-write/Cargo.toml` is now `0.14.8`, eliminating the
  duplicate-version split.

### Fixed

- **ZSTD correctness regression** — three ZSTD tests
  (`zstd_higher_levels_compress_better_than_lower`,
  `zstd_compresses_binary_data`,
  `zstd_compresses_better_than_lz4_on_text`) had been silently
  hanging or producing pathological output because omnizip 0.14.6/0.14.8
  has a regression at `ZstdLevel::Default` (L6) and higher on
  highly-repetitive inputs. The encoder produces 50 KB of effectively
  uncompressed output for 90 KB of repeated text that L1 compresses
  to 74 bytes, and takes 14+ seconds to do it.

  LimniFS now caps the ZSTD level at `Fast` (L3) — see
  `docs/omnizip-proposals/zstd-default-broken.md` for the upstream
  bug report and acceptance criteria. Decompression is unaffected
  (ZSTD's wire format is level-independent). When omnizip fixes L6,
  restore the `6..=11 → Default, 12..=21 → Better, 22+ → Best`
  mapping in `limnifs-core/src/codec/zstd.rs::level_for_quality`.

  Workspace test suite runtime: 367 s → 8 s (45× faster).

## [0.2.20] — 2026-08-05

### Added

- **Multi-profile benchmark** — `limnifs-bench run --profile
  balanced,max-write,max-ratio` exercises the full benchmark matrix
  for each requested LimniFS profile. Each profile produces separate
  rows in the report (format tag `limnifs:{profile}`). External
  formats (DwarFS, SquashFS, tar+zstd) run once per dataset. Closes
  TODO.perf/03.

  The report renderer now derives the format list dynamically from
  results, so future profiles appear in tables and the win/loss
  matrix without code changes.

## [0.2.19] — 2026-08-05

### Added

- **madvise(MADV_WILLNEED) prefetch** — `SlabStore::load_mmap` now
  hints the kernel to prefetch slab pages after mmap. On cold cache,
  pages are readahead'd in parallel rather than faulted one at a
  time. Closes TODO.perf/05.

- **skip_chunking field** — `WriteConfig::skip_chunking: bool`
  (default false). When true, `process_file` compresses each file
  as a single drop without FastCDC hashing — **19–28% faster
  create** on incompressible/large data. `max_write()` and
  `max_write_rw()` profiles set it to true. Balanced/max-ratio
  profiles keep it false (chunking enables dedup).

### Benchmark impact (max_write + skip_chunking vs previous)

| Dataset | Before | After | Speedup |
|---|---:|---:|---:|
| Random (100 MB) | 0.289s | 0.235s | **19%** |
| Zeros (100 MB) | 0.148s | 0.106s | **28%** |

### Test count

575 (unchanged).

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

[0.2.19]: https://github.com/limnifs/limnifs/releases/tag/v0.2.19
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
