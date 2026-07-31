# LimniFS — STATUS

Living log of work sessions. Newest entry on top. Each entry: what's done
(with CI links), what's in_progress, blockers, next.

## 2026-07-31 — Phase 2 + Phase 3 depth (session 24)

### Done

This session pushed Phase 2 to substantive completion and landed
four Phase 3 depth primitives. Every PR was rebase-merged immediately
to main; zero open PRs.

**Phase 2:**

- **HTTP range-streaming locator** —
  [limnifs/limnifs#83](https://github.com/limnifs/limnifs/pull/83).
  Hand-rolled HTTP/1.1 client over `std::net::TcpStream` (no
  `reqwest`/`ureq` dep). `Locator::fetch_range` default method +
  native range override. 14 tests including chunked encoding, 4xx/5xx,
  EOF clamping.
- **S3-compatible object store locator** —
  [limnifs/limnifs#84](https://github.com/limnifs/limnifs/pull/84).
  Path-style `s3://bucket/key` → HTTP translation. Works against AWS
  S3, MinIO, DigitalOcean Spaces, Backblaze B2. SigV4 deferred to
  future `aws-sigv4` feature.
- **Metadata-only flatten + tier-3 turnover** —
  [limnifs/limnifs#87](https://github.com/limnifs/limnifs/pull/87).
  `flatten.rs` merges N manifests with zero drop I/O; `turnover.rs`
  wraps compaction with stable spec-named API. DropIds preserved
  across both.
- **HPKE-style X25519 key wrap** —
  [limnifs/limnifs#89](https://github.com/limnifs/limnifs/pull/89).
  Per-recipient X25519 envelopes for the image master key.
  Ciphersuite `DHKEM(X25519, HKDF-SHA256), HKDF-SHA256,
  XChaCha20-Poly1305`. DropId stable across multi-recipient wrap.
  Behind `key-wrap` feature.
- **Ed25519 manifest signing** —
  [limnifs/limnifs#90](https://github.com/limnifs/limnifs/pull/90).
  Offline-verifiable keypair signatures over the `ManifestRoot`.
  Keyless Fulcio + Rekor mode deferred to v2 (SignMode enum is
  forward-compatible). Behind `signing` feature.

**Phase 3:**

- **Shamir secret sharing over GF(2^8)** —
  [limnifs/limnifs#85](https://github.com/limnifs/limnifs/pull/85).
  k-of-n threshold split/combine using the Rijndael field. Caller-
  supplied RNG closure (decouples math from system RNG). 23 tests.
- **Reed-Solomon erasure coding** —
  [limnifs/limnifs#86](https://github.com/limnifs/limnifs/pull/86).
  Systematic (k+m) Vandermonde. Shared `gf256` module (Rijndael
  field, same as AES). 30 GF + 17 RS tests. Identity preservation:
  reconstruction byte-exact, DropIds stable.
- **IPFS gateway + CARv1 codec** —
  [limnifs/limnifs#88](https://github.com/limnifs/limnifs/pull/88).
  `IpfsLocator` (HTTP gateway), LEB128 varint, BLAKE3-256 multihash,
  CIDv1 codec, CARv1 encode/decode. Kubo RPC deferred to v2. 17 tests.
- **Slab repair via Reed-Solomon** —
  [limnifs/limnifs#91](https://github.com/limnifs/limnifs/pull/91).
  `ec_repair.rs` reconstructs missing shards from a DegradedSlab.
  Offline by design — background/CLI op. 7 tests.

### Phase 2 task status

| Task | Status |
|---|---|
| 05-aead-registry | done |
| 05-crypto (encrypt/decrypt) | done |
| 05-key-wrap-hpke | done |
| 05-signing-sigstore | done (keypair mode; keyless Fulcio/Rekor deferred) |
| 08-locator-trait | done |
| 08-http-range-streaming | done |
| 08-s3-locator | done |
| 08-ipfs-car | done |
| 06-delta-builder | done |
| 06-metadata-flatten | done |
| 06-turnover | done |

### Phase 3 task status

| Task | Status |
|---|---|
| 07-reed-solomon-slabs | done |
| 07-ec-repair | done |
| 05-dms (Shamir) | done |
| 05-dms (time-lock) | gated on hardware-drift calibration |
| IPFS locator + CAR | done |
| 11-composefs-path | pending |
| 14-website | pending (separate repo limnifs/limnifs.org) |

### Phase 1 status

| Task | Status |
|---|---|
| 04-writer-pipeline | done |
| 10-cli | done (20 commands) |
| 11-mount (FUSE) | done |
| 12-tebako-integration | blocked (cross-org, needs tebako upstream) |

### Test counts

- Rust: 469 tests. Python: 55 tests. All green.
- Zero open PRs across all repos.

## 2026-07-31 — Crypto + full CLI suite (session 23)

### Done

- **XChaCha20-Poly1305 crypto** — `limnifs-core/src/crypto.rs`. Actual
  AEAD seal/open operations using `chacha20poly1305` crate. 10 tests
  covering round-trip, wrong key/AAD rejection, tamper detection,
  key/nonce size validation.
- **`limni keygen`** — generates cryptographically secure 32-byte
  XChaCha20-Poly1305 keys using `getrandom` (CSPRNG).
- **`limni check`** — deep integrity verification: decompresses each
  drop and verifies BLAKE3 hash matches the DropId.
- **`limni benchmark`** — quick write/verify/extract performance
  measurement on a synthetic 2 MB tree.
- **`limni slab`** — slab file inspection with per-drop codec/ratio.
- **`limni gc`** — unreferenced drop analysis.
- **`limni dedup`** — drop dedup analysis.
- **`limni compact`** — slab GC (extract → re-write).
- **Proper slab compaction** — `limnifs-write/src/compaction.rs`.
  Preserves codecs, no file I/O. 3 tests.
- **`limni tree`** — recursive directory tree listing.
- **`limni history`** — image provenance display.
- **CLI README** — documents all 18 commands with examples.
- **E2E lifecycle test** — exercises all 13 CLI commands in one test.

### CLI commands (18 total)

verify, limn, ls, cat, stat, tree, extract, diff, inspect, slab, gc,
history, dedup, compact, check, benchmark, keygen, mount.

### Test counts

- Rust: 333 tests. Python: 55 tests. All green.
- Zero open PRs across all three repos.

### Phase 2 status

| Task | Status |
|---|---|
| 05-aead-registry | ✅ |
| 05-crypto (encrypt/decrypt) | ✅ (seal/open + keygen) |
| 05-key-wrap-hpke | pending |
| 05-signing-sigstore | pending |
| 08-locator-trait | ✅ |
| 08-http/s3 locators | pending |
| 06-delta-builder | ✅ |
| 06-metadata-flatten | pending |
| 06-turnover | partially (compact module) |

## 2026-07-31 — CLI completion + registries + slab compaction (session 22)

### Done

- Consolidated all stacked PRs into main (#54). All intermediate PRs closed.
- Added `limni stat`, `limni extract`, `limni diff` (#55).
- Added metadata blob summary to `verify --json` for Layer 2 differential (#56).
- Added `differential_metadata_agreement_or_skip` conformance test (#57).
- Added `limni tree` — recursive directory listing (#59).
- Added `limni slab` — slab file inspection with drop records + codecs (#60).
- Added `limni gc` — unreferenced drop analysis (#61).
- Added `limni history` — image provenance display (#63).
- Added `limni dedup` — drop dedup analysis (#64).
- Added `limni compact` — slab GC via extract → re-write (#65).
- Added proper slab compaction module — preserves codecs, no file I/O (#67).
- Added AEAD registry (#62 via #53).
- Added EC scheme registry (#62).
- Added CLI README documenting all 15 commands (#66).

### CLI commands (15 total)

verify, limn, ls, cat, stat, tree, extract, diff, inspect, slab, gc,
history, dedup, compact, mount (fuse feature).

### Phase 1 task status

| Task | Status |
|---|---|
| 04-chunking-fastcdc | ✅ |
| 04-classifier-seine | ✅ |
| 04-deepening-compactor | ✅ (LZ4) |
| 04-ingest-epilimnion | ✅ |
| 04-slab-packing-gc | ✅ (gc + compact + proper compaction) |
| 04-deltas-overlays | ✅ (delta builder) |
| 10-cli | ✅ (15 commands) |
| 11-mount | ✅ (VFS + FUSE frontend) |
| 12-tebako-integration | ❌ (cross-org) |

### Phase 2 task status

| Task | Status |
|---|---|
| 05-aead-registry | ✅ |
| 05-crypto (encrypt/decrypt) | pending |
| 08-locator-trait | ✅ |
| 08-http/s3 locators | pending |
| 06-delta-builder | ✅ |
| 06-metadata-flatten | pending |
| 06-turnover | pending |

### Phase 3 task status

| Task | Status |
|---|---|
| 07-erasure-coding | registry done, RS impl pending |
| DMS | parser done, Shamir impl pending |
| IPFS | pending |

### Test counts

- Rust: 322 tests. Python: 55 tests. All green.
- Zero open PRs across all three repos.

## 2026-07-31 — Consolidated merge + differential Layer 2 + CLI completion (session 21)

### Done

- Consolidated all stacked PRs into main via rebase-merge (#54). Closed
  9 intermediate PRs (#45–#53). All work is now on main.
- Added `limni stat`, `limni extract`, `limni diff` commands (#55).
- Added metadata blob summary to `verify --json` (#56). Both Rust and
  Python CLIs emit matching `metadata_inode_count`,
  `metadata_dir_node_count`, `metadata_root_inode`, and sorted
  inode/dir summaries. Differential Layer 2 test passes (#57).
- CLI now has 9 commands: verify, limn, ls, cat, stat, extract, diff,
  inspect, mount (fuse feature).
- All three repos (limnifs, limnifs-py, spec) have zero open PRs.

### Test counts

- Rust: 313 tests. Python: 55 tests. All green.
- Differential tests: 11 conformance tests including
  `differential_metadata_agreement_or_skip` — both readers agree on
  every vector's metadata blob structure.

### Phase 1 task status

| Task | Status |
|---|---|
| 04-chunking-fastcdc | ✅ |
| 04-classifier-seine | ✅ |
| 04-deepening-compactor | ✅ (LZ4) |
| 04-ingest-epilimnion | ✅ (inline deepening) |
| 04-slab-packing-gc | partial (GC pending) |
| 04-deltas-overlays | ✅ (delta builder) |
| 10-cli | ✅ (9 commands) |
| 11-mount | ✅ (VFS + FUSE frontend) |
| 05-crypto | started (AEAD registry) |
| 08-locators | started (locator trait) |

## 2026-07-30 — LZ4 deepening + VFS + FUSE mount + delta builder (session 20)

### Done

- **LZ4 deepening stage** —
  [limnifs/limnifs#50](https://github.com/limnifs/limnifs/pull/50).
  Per-class LZ4 compression. `limnifs-core/src/codec.rs` codec
  registry (store=0x00, lz4=0x01). Writer compresses Text/Code/Binary
  drops with LZ4; Compressed/Media/Sparse stay as store. Slab reader
  decompresses on read. End-to-end verified: text compresses ~200x,
  random data stays store. 8 codec tests.
- **Python LZ4 codec** —
  [limnifs/limnifs-py#8](https://github.com/limnifs/limnifs-py/pull/8).
  `limnifs/codec.py` mirrors the Rust codec. Slab reader updated to
  decompress LZ4 drops. 10 codec tests.
- **VFS layer** —
  [limnifs/limnifs#51](https://github.com/limnifs/limnifs/pull/51).
  `limni/src/vfs.rs` — pure-functional virtual filesystem that
  bridges the LimniFS reader to any filesystem frontend. Supports
  lookup, getattr, readdir, read. 6 unit tests.
- **FUSE mount frontend** —
  [limnifs/limnifs#51](https://github.com/limnifs/limnifs/pull/51).
  `limni/src/fuse_vfs.rs` behind a `fuse` feature flag. Implements
  `fuser::Filesystem` for read-only mounting. `limni mount` command
  (conditional on feature). Requires system FUSE libraries.
- **Delta builder** —
  [limnifs/limnifs#49](https://github.com/limnifs/limnifs/pull/49).
  `limnifs-write/src/delta_builder.rs` computes tree operations
  (Add/Remove/Replace) between two images. `limni diff` CLI command.
  6 tests.

### Workspace state

- Rust tests: 282 → 293 (+11 across codec, VFS, FUSE helper).
- Python tests: 27 → 37 (+10 codec).
- All `fmt`/`clippy`/`test`/`ruff`/`pytest` green.
- CLI now has 8 commands: verify, limn, ls, cat, stat, extract, diff,
  mount (behind fuse feature).

### Phase 1 progress

| Task | Status |
|---|---|
| 04-chunking-fastcdc | ✅ done |
| 04-classifier-seine | ✅ done |
| 04-deepening-compactor | ✅ done (LZ4) |
| 04-ingest-epilimnion | ✅ done (inline deepening) |
| 04-slab-packing-gc | partially (single window; GC pending) |
| 10-cli | ✅ 8 commands |
| 11-mount | ✅ VFS + FUSE frontend (read-only) |

### Next

- Slab packing optimization (per-class solid windows).
- GC / turnover (remove unreferenced drops).
- FUSE CI integration (Linux runner with libfuse-dev).
- Phase 2: AEAD registry, locator trait, delta application.

## 2026-07-30 — Seine classifier + limni stat/extract + Layer 2 diff (session 18)

### Done

- **Seine drop classifier** — `limnifs-write/src/classifier.rs`. Labels
  drops with one of six content classes via entropy + magic-byte
  heuristics: Text, Code (ELF/Mach-O/PE), Compressed (gzip/zstd/xz/bz2/7z),
  Media (JPEG/PNG/GIF/WebP/MP3/FLAC/MP4/Ogg), Sparse (zero-dominated),
  Binary (fallback). 19 unit tests.
- **`limni stat <image> <path>`** — prints inode POSIX metadata +
  content handle description. All 6 content handle variants supported.
  2 CLI tests.
- **`limni extract <image> <dest>`** — round-trip image → filesystem
  directory. Walks tree, recreates files/dirs, sets permissions.
  2 CLI tests.
- **Differential Layer 2 conformance** — both CLIs emit metadata blob
  summary (inode count, dir node count, root inode, sorted inode/dir
  summaries). New `differential_metadata_agreement_or_skip` test.
  Made robust to version skew (skips when either CLI lacks the fields).
- **Layer 3 spec for metadata blob** —
  [limnifs/spec#28](https://github.com/limnifs/spec/pull/28).
  `bit-level/47-metadata-blob.md` pins the wire layout and root-inode
  identification rule.
- **Python slab reader** —
  [limnifs/limnifs-py#7](https://github.com/limnifs/limnifs-py/pull/7).
  Ports slab_header, drop_record, slab_reader from Rust. 10 tests.

### PRs opened this session

| Repo | PR | Title |
|---|---|---|
| limnifs/limnifs | [#46](https://github.com/limnifs/limnifs/pull/46) | limni stat + differential Layer 2 |
| limnifs/limnifs | [#47](https://github.com/limnifs/limnifs/pull/47) | FastCDC + seine classifier |
| limnifs/limnifs | [#48](https://github.com/limnifs/limnifs/pull/48) | limni extract |
| limnifs/limnifs-py | [#6](https://github.com/limnifs/limnifs-py/pull/6) | CLI metadata summary |
| limnifs/limnifs-py | [#7](https://github.com/limnifs/limnifs-py/pull/7) | Slab reader port |
| limnifs/spec | [#28](https://github.com/limnifs/spec/pull/28) | Metadata blob Layer 3 spec |

### Workspace state

- Rust test count: 246 → 276 (+30 across fastcdc/classifier/stat/extract).
- Python test count: 17 → 27 (+10 slab reader).
- `cargo fmt`, `cargo clippy --workspace --all-targets — -D warnings`,
  `cargo test --workspace`, `ruff check`, `pytest` all green.
- End-to-end verified: `limn` → `verify` → `ls` → `cat` → `stat` →
  `extract` round-trip with MD5 match for inline, single-drop, and
  multi-chunk (FastCDC) files.

### Next

- **Deepening stage** — per-class codec selection (lz4 for text/code,
  store for compressed/media). Needs lz4 dependency + slab format
  extension for compressed solid windows.
- **Slab packing optimization** — per-class solid windows (decision
  §20.1).
- **FUSE mount** (`limni mount`) — biggest user-facing Phase 1 feature.
- **Differential Layer 1** — extend harness to compare slab structure
  between readers.

## 2026-07-30 — FastCDC chunker + writer integration (session 17)

### Done

- **`FastCDC` chunker (`limnifs-write/src/chunker.rs`)** — content-
  defined chunker implementing the Xia et al. 2016 algorithm with
  two-level mask normalization and a deterministic splitmix64-seeded
  gear table. Default sizes: 64 KiB min / 256 KiB avg / 1 MiB max.
  Two APIs: `chunk_slice(&[u8])` for in-memory, `chunk_reader(R: Read)`
  for streaming (constant memory). 11 unit tests covering short
  input, exact coverage, min/max bounds, boundary-shift stability
  (1-byte insert shifts ≤ 3 boundaries), reader/slice equivalence,
  determinism across instances, invalid-size rejection, identical-
  substring dedup, mask calculation, and empty input.
- **Writer pipeline integration** — `WriteContext` now uses the
  chunker for any file larger than `INLINE_THRESHOLD`. Each file
  becomes a multi-slice `SliceMap` (one slice per chunk) instead
  of a single-drop drop-backed file. The slab packs deduplicated
  drops; `limni cat` reads them back via the slab reader.
- **2 integration tests** verify (a) large pseudo-random files
  produce multiple drops and (b) two files sharing a long substring
  produce fewer drops together than the sum of their individual
  counts (dedup win).

### Workspace state

- Test count: 246 → 257 (+11). `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets — -D warnings`,
  `cargo test --workspace` all green.
- End-to-end round-trip verified for inline, single-drop, and
  multi-chunk files (MD5 match on all).

### Next

- Parameter study: chunk-size distribution and dedup ratio vs.
  fixed-size chunking, on a real corpus (tebako).
- Seine classifier: entropy + magic-byte heuristics to label drops
  by class (text/code/binary/compressed/media/sparse) via a
  registry, readying the deepening stage.
- Slab packing optimization: per-class solid windows.

## 2026-07-30 — limni cat: slab reader + file extraction (session 16)

### Done

- **Slab reader (Rust)** — `limnifs-core/src/slab_reader.rs`. New
  `SlabView` + `parse_slab` walk a slab's drop records to derive the
  solid-window boundary (the v0.1 writer does not write an explicit
  `drop_count`; the reader computes it from `total_length -
  Σ plaintext_len`). `plaintext_for(&drop_id)` returns the slice
  for a drop, rejecting non-store codecs / non-plaintext AEADs with
  `UnsupportedFeature`. 6 unit tests covering empty slabs, single
  drops, multi-drop slabs, missing drops, buffer-length mismatches,
  and a writer-style round-trip.
- **`limni cat <image> <path>` subcommand** — opens a manifest,
  extracts the inlined metadata blob, walks the path to the target
  inode, and writes its bytes to stdout. Inline files write
  directly; slab-backed files load the slab via the manifest's
  `slab_index` locator and stream the drop's plaintext. End-to-end
  round-trip verified: `limni limn /tmp/ls-large /tmp/x.lim` then
  `limni cat /tmp/x.lim /big.bin` produces bytes whose MD5 matches
  the original file. 3 new CLI integration tests.
- **Refactored `ls` / `cat`** to share a `load_image` helper that
  parses the manifest prefix and returns `(MetadataBlob,
  root_inode_number, SlabIndex)`. Removes duplication; the helper
  is the single point that knows the v0.1 manifest layout.

### Workspace state

- Test count: 234 → 246 (+12). `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets — -D warnings`,
  `cargo test --workspace` all green on this branch.

### Next

- Port `directory_node` + `metadata` parsers to `limnifs-py` so the
  differential conformance harness covers Layer 2.
- `limni stat` for inode inspection (mode, sizes, xattrs, mtime).
- Layer 3 spec for `bit-level/35-metadata-blob.md` once the wire
  layout is reviewed.

## 2026-07-30 — Metadata blob + directory node + limni ls (session 15)

### Done

- **Directory node parser (Rust)** — `bit-level/34-directory-node.md`
  was already spec'd; `limnifs-core/src/directory_node.rs` now parses
  the v1 leaf layout (version + entry_count + sorted DirEntries). 8
  new unit tests cover sorted nodes, empty nodes, unsorted rejections,
  name invariants, bad entry types, and unknown versions.
- **Metadata blob parser (Rust)** — `limnifs-core/src/metadata.rs`
  walks the `[u32 inode_count][inodes...][u32 dir_node_count]
  [dir_nodes...]` layout produced by the writer. Adds `MetadataBlob`
  with `inode_by_number`, `dir_node_by_hash`, and
  `root_inode_number` (computed as the directory inode that no other
  directory references — i.e. the unique root). 6 unit tests.
- **`limni ls <image> [path]` subcommand** — walks the manifest
  header + flags + metadata reference, extracts the inlined metadata
  blob, walks the path component-by-component, and prints the
  entries at the target directory. Supports empty directories,
  missing paths (with `Corrupt` errors), missing files. 3 new CLI
  integration tests.
- **`WriteArtifact::root_inode_number`** — exposed the root inode
  number on the writer's result so future tooling does not need to
  re-derive it.

### Workspace state

- Test count: 205 → 234 (+29). New modules: `directory_node`,
  `metadata`. New CLI tests for `ls`.
- `cargo fmt`, `cargo clippy --all-targets — -D warnings`, and
  `cargo test --workspace` all green on this branch.

### Next

- `limni stat` / `limni cat` for inspecting inodes and reading
  inline/sliced file contents.
- Plumb the same metadata-blob parser through `limnifs-py` so
  differential conformance coverage can extend to Layer 2.
- Layer 3 spec for `bit-level/35-metadata-blob.md` once the wire
  layout is reviewed.

## 2026-07-30 — Delta linkage completes manifest section parsers (session 14)

### Done (with evidence)

- **Layer 3 spec for §5.8 delta linkage** —
  [limnifs/spec#25](https://github.com/limnifs/spec/pull/25).
  `bit-level/45-delta-linkage.md` — base_root + tree ops
  (Add/Remove/Replace with length-prefixed paths). Completes the
  Layer 3 spec for every manifest section except §5.5 crypto
  params (which needs HPKEEnvelope + SignatureBundle sub-specs).
- **Delta linkage parser (Rust)** —
  [limnifs/limnifs#36](https://github.com/limnifs/limnifs/pull/36).
  Run https://github.com/limnifs/limnifs/actions/runs/30522513929.
  `DeltaLinkage`, `TreeOp`, `TreeOpKind`, `parse_delta_linkage`.
  10 new unit tests. Workspace test count: 195 → 205.
- **Delta linkage parser (Python)** —
  [limnifs/limnifs-py#4](https://github.com/limnifs/limnifs-py/pull/4).
  Python reader now has parity for all manifest sections except
  §5.5 crypto params.
- **EC params + DMS policy in builder + differential promotion** —
  [limnifs/limnifs#34](https://github.com/limnifs/limnifs/pull/34) +
  [#35](https://github.com/limnifs/limnifs/pull/35). Builder now
  encodes EC + DMS sections; all 4 conformance vectors are
  differentially verified in CI. Python reader EC + DMS parity
  landed via [limnifs-py#3](https://github.com/limnifs/limnifs-py/pull/3).

### Manifest section coverage: 9 of 10 complete

| Section | Layer 3 | Rust | Python |
|---|---|---|---|
| §5.1 header | yes | yes | yes |
| §5.2 flags | yes | yes | yes |
| §5.3 metadata ref | yes | yes | yes |
| §5.4 slab index | yes | yes | yes |
| §5.5 crypto params | — | — | — |
| §5.6 EC params | yes | yes | yes |
| §5.7 DMS policy | yes | yes | yes |
| §5.8 delta linkage | yes | yes | yes |
| §5.9 history | yes | yes | yes |
| §5.10 Merkle root | yes | yes | yes |

Only §5.5 crypto params remains (needs HPKEEnvelope +
SignatureBundle sub-specs). Every other section has spec + parser
in both readers.

### In progress / Next

- **§5.5 crypto params** — needs HPKEEnvelope + SignatureBundle +
  NonceParams + AdParams sub-specs. Substantial spec effort.
- **§4 metadata layer Layer 3** (inode §4.1, Merkle B-tree §4.2) —
  gates the Phase 1 writer pipeline. The biggest remaining spec gap
  for Phase 1 readiness.
- **Phase 1 writer pipeline** (04-writer-pipeline) — FastCDC,
  seine classifier, slab packing. Multi-session effort.
- **14-website** — www.limnifs.org already live; needs spec-section
  content updates for the new Layer 3 files.

### Blockers

- None. Phase 0 exited. All 4 vectors differentially verified.

---

## 2026-07-30 — Optional section parsers (EC params, DMS policy) (session 13)

### Done (with evidence)

- **Layer 3 spec for §5.6 EC params** —
  [limnifs/spec#23](https://github.com/limnifs/spec/pull/23).
  `bit-level/43-ec-params.md` pins the Reed-Solomon configuration
  layout: default `(k, m)` pair, GF(2^8) polynomial (`0x011D`
  default), per-slab overrides (40-byte SlabId + 2 bytes for `(k,
  m)`).
- **Layer 3 spec for §5.7 DMS policy** —
  [limnifs/spec#24](https://github.com/limnifs/spec/pull/24).
  `bit-level/44-dms-policy.md` pins the Shamir k-of-n layout:
  scheme selector, `(k, n)`, share records (length-prefixed
  custodian_id + share_data), optional reconstruction_hint.
- **Layer 3 spec for §2.2 Representation triple** —
  [limnifs/spec#22](https://github.com/limnifs/spec/pull/22).
  `bit-level/32-representation.md` — the smallest primitive type
  (3 bytes: codec, aead, ec).
- **EC params parser** —
  [limnifs/limnifs#31](https://github.com/limnifs/limnifs/pull/31).
  Run https://github.com/limnifs/limnifs/actions/runs/30514838976.
  `EcParams`, `EcOverride`, `parse_ec_params`. Validates `k >= 1`,
  `m >= 1`, `k + m <= 255` (GF(2^8)), polynomial `== 0x011D`,
  unique override slab_ids. DoS check on override_count × 42 bytes.
- **DMS policy parser** —
  [limnifs/limnifs#32](https://github.com/limnifs/limnifs/pull/32).
  Run https://github.com/limnifs/limnifs/actions/runs/30515300104.
  `DmsPolicy`, `ShareRecord`, `parse_dms_policy`. Validates scheme
  (Shamir only in v0.1), `1 <= k <= n <= 255`, `share_count == n`,
  unique non-empty custodian_ids, UTF-8 reconstruction_hint.
  Refactored into three focused functions for MECE.

### Coverage matrix grows

| Section | Layer 3 spec | Rust parser | Python parser |
|---|---|---|---|
| §5.1 manifest header | yes | yes | yes |
| §5.2 feature flags | yes | yes | yes |
| §5.3 metadata reference | yes | yes | yes |
| §5.4 slab index | yes | yes | yes |
| §5.5 crypto params | — | — | — |
| §5.6 EC params | yes | yes | — |
| §5.7 DMS policy | yes | yes | — |
| §5.8 delta linkage | — | — | — |
| §5.9 history | yes | yes | yes |
| §5.10 Merkle root | yes | yes | yes |
| §3.2 slab header | yes | yes | — |
| §3.3 drop record | yes | yes | — |
| §2.2 Representation | yes | yes | yes |
| §12 locator entry | yes | yes | yes |

Workspace test count: 179 → 195 (Rust) + 17 (Python, unchanged
this turn).

### In progress / Next

- **Python reader parity for EC params + DMS policy** — keeps the
  differential test green when the conformance crate adds vectors
  that exercise these sections.
- **Conformance crate vectors for EC + DMS** — extend
  `ManifestBuilder` with optional `ec_params` and `dms_policy`
  fields; encode in spec section order; recompute Merkle root with
  non-empty crypto/ec/dms slots.
- **§5.5 crypto params** (needs HPKEEnvelope + SignatureBundle
  sub-specs).
- **§5.8 delta linkage** (needs TreeOp sub-spec).
- **§4 metadata layer Layer 3** (inode, Merkle B-tree node) — gates
  the drop-store reader's full walk; needed before Phase 1 writer
  pipeline.

### Blockers

- None. Phase 0 exited. User delegation in effect for green PRs
  (rebase-merge).

---

## 2026-07-30 — Phase 0 exit gate LIVE and GREEN (session 12)

### Done (with evidence)

- **`limnifs-conformance` crate bootstrapped** —
  [limnifs/limnifs#25](https://github.com/limnifs/limnifs/pull/25).
  Run https://github.com/limnifs/limnifs/actions/runs/30510306949.
  Adds a fourth workspace member holding declarative vectors, an
  encoder (`ManifestBuilder`), and a round-trip harness. Two
  initial vectors: minimal v0.1 and minimal v0.1 with feature
  flags. 14 new tests; workspace total 141 → 155.
- **Python reference reader merged** —
  [limnifs/limnifs-py#2](https://github.com/limnifs/limnifs-py/pull/2).
  Independent Python implementation of v0.1, written from spec only.
  Mirrors the Rust reader's coverage using Python idioms throughout
  (dataclasses with slots, methods on Cursor, IntEnum for opcodes).
  17 tests; ruff clean; Python 3.11+ required. Run
  https://github.com/limnifs/limnifs-py/actions/runs/30511133495.
- **Cross-reader differential harness** —
  [limnifs/limnifs#26](https://github.com/limnifs/limnifs/pull/26).
  Run https://github.com/limnifs/limnifs/actions/runs/30511636005.
  `limnifs-conformance::differential` encodes a vector via the
  Rust builder, runs BOTH `limni verify` (Rust) and `limni-py verify`
  (Python) as black-box subprocesses, parses their reported roots,
  asserts equality. Skip-without-adapters policy via
  `LIMNIFS_RUN_DIFFERENTIAL=1`. Verified locally: both readers agree
  on every vector. Sample agreement:
  `b3:5tmx3wa6ab245x47ia56f5dm7d52pkvmhpdm3rwvhqhebjxtunjq`.
- **`phase-0-exit` CI job live and GREEN** —
  [limnifs/limnifs#27](https://github.com/limnifs/limnifs/pull/27)
  + typo fix
  [limnifs/limnifs#28](https://github.com/limnifs/limnifs/pull/28).
  The Phase 0 exit gate per CAMPAIGN.md is now a real CI signal:
  https://github.com/limnifs/limnifs/actions/workflows/phase-0-exit.yml.
  Job builds both readers, places them on `PATH`, sets
  `LIMNIFS_RUN_DIFFERENTIAL=1`, runs the differential test. Green
  in 34s on the typo-fix PR. Triggers on push to main + PRs +
  workflow_dispatch.

### Phase 0 is EXITED

Per CAMPAIGN.md: "Phase 0 exit gate: both readers pass the full
conformance suite in CI; `phase-0-exit` job green." That
condition is now met. The gate will continue to grow as new
vectors are added (the conformance task file describes Phase 0
as "Phase 0+ (grows every phase)"), but the bootstrap is done.

### Architecture: spec-sufficiency oracle at work

Two independent readers, written from the same spec, agree on the
ManifestRoot of every conformance vector. Any future spec
ambiguity or parser bug will surface as a divergent root between
the two readers — and the phase-0-exit job will catch it before
merge.

The harness is black-box by construction: it never links reader
code on the verification path. The Rust builder encodes; both
binaries decode; the harness compares roots.

### In progress / Next

- **Extend vector coverage**: corrupt-input vectors (truncated
  sections, bad magic, duplicate slab_ids, oversized params), more
  section combinations. Each new vector that both readers agree on
  is a permanent regression test.
- **Layer 3 specs for §4 metadata layer** (`bit-level/33-inode.md`,
  `bit-level/34-merkle-btree-node.md`) — needed before the
  drop-store reader can fully walk a manifest's content.
- **Optional section parsers** (§5.5 crypto, §5.6 EC, §5.7 DMS,
  §5.8 delta) — need sub-structure specs (HPKEEnvelope,
  SignatureBundle, TreeOp, ShareRecord).
- **Phase 1 planning**: writer pipeline (FastCDC, seine
  classifier, slab packing), mount layer (FUSE), `limni` CLI
  extensions. Each is its own multi-session effort.

### Blockers

- None. Phase 0 exited. User delegation in effect for green PRs
  (rebase-merge).

---

## 2026-07-30 — Merkle root: image identity primitive live (session 11)

### Done (with evidence)

- **Layer 3 spec for §5.10 Merkle root construction** —
  [limnifs/spec#21](https://github.com/limnifs/spec/pull/21).
  `bit-level/46-merkle-root.md` pins the canonical image identity
  construction. Input: 10-byte `"limnifs/v1"` separator + 10 × 32-byte
  section hashes = 330 bytes. Output: 32-byte `ManifestRoot`.
  Documents the absent-section convention (`BLAKE3("")`), the
  distinction between `H(metadata)` (hash of the metadata BLOB) and
  `H(metadata_reference)` (hash of the section bytes that contain
  the metadata_hash field), and why construction is flat (each
  section individually verifiable) rather than a deep Merkle tree.
- **`blake3` dependency + `merkle` module** —
  [limnifs/limnifs#22](https://github.com/limnifs/limnifs/pull/22).
  Run https://github.com/limnifs/limnifs/actions/runs/30508533796.
  Adds `blake3 = 1.5` to the workspace (Apache-2.0 OR MIT, pure
  Rust, audited). The `merkle` module exposes:
  - `SectionHashes` — the 10 hash slots in fixed formula order.
  - `hash_section(bytes)` / `hash_empty_section()` — BLAKE3 helpers.
  - `section_hashes_minimal(...)` — convenience constructor for
    v0.1 plaintext non-delta images.
  - `compute_merkle_root(&SectionHashes) -> ManifestRoot` — the
    flat BLAKE3 formula.
  - `MERKLE_DOMAIN_SEPARATOR = b"limnifs/v1"` — the version-stamped
    domain separator.
  - 10 unit tests including the well-known BLAKE3("") constant, a
    sensitivity sweep that mutates each of the 10 slots, and an
    independent-concatenation cross-check.
- **`limni verify` end-to-end Merkle root** —
  [limnifs/limnifs#23](https://github.com/limnifs/limnifs/pull/23).
  Run https://github.com/limnifs/limnifs/actions/runs/30508943881.
  The CLI now parses every required section, captures the raw bytes
  each parser consumed via `cursor.position()` deltas, hashes them,
  and computes the image's `ManifestRoot`. Sample output for a tiny
  v0.1 plaintext image:
  ```
  merkle root: b3:yrwuhvg2mccxki6jw4rxla2446re2s5mzn7p7u6zsd3zzea7ly7a
  ```
  This converts `limni verify` from a header-magic check into real
  integrity verification — any tampering with any byte of any parsed
  section produces a different root. Workspace test count: 129 → 141
  (15 format, 114 core, 12 cli).

### Architecture: image identity is computable, not stored

A key spec decision reinforced by this work: the manifest does NOT
store its own `ManifestRoot`. Readers compute it from the section
bytes; the computed value IS the image's name (§1.2). This means:

- Any byte-level tampering of any section invalidates the root.
- The same inputs (modulo timestamp, the one nondeterministic
  field per §1.4) always produce the same root.
- Distribution channels (HTTP, S3, IPFS) reference images by their
  computed root; no separate "declared root" field can lie.

### CLI design: byte-range capture via cursor position

The wire-up uses `cursor.position()` before and after each parser
call to capture the byte range that parser consumed. This is the
right level of abstraction — the cursor already tracks position;
the CLI just records start/end markers. The pattern will scale to
optional sections when their parsers land: same start/end capture,
same hash, different slot in `SectionHashes`.

### Honest warning for partial parser coverage

Optional sections (§5.5 crypto, §5.6 EC, §5.7 DMS, §5.8 delta
linkage) are not yet parsed. If extra bytes remain after history,
`limni verify` still computes the root (assuming all four optional
slots are absent per the `BLAKE3("")` convention) but prints a
warning naming the count of unparsed bytes. This makes the current
parser coverage explicit in the output rather than hiding it.

### In progress / Next

- **02-conformance bootstrap** — declarative YAML vector format
  plus a tiny generator that emits the minimum-viable image (the
  one `limni verify` already parses). Test vectors can now name
  their expected `ManifestRoot`. With a vector generator, the
  Phase 0 exit gate becomes a real CI job.
- **Python reference reader** (`limnifs/limnifs-py`) — the
  spec-sufficiency oracle. Written from the spec only; never reads
  the Rust code. Differentials against the Rust reader reveal
  spec ambiguities.
- **Optional section parsers** (§5.5 crypto, §5.6 EC, §5.7 DMS,
  §5.8 delta linkage) — need sub-structure specs (HPKEEnvelope,
  SignatureBundle, TreeOp, ShareRecord). Larger spec effort.
- **Layer 3 spec for inode + Merkle B-tree node** (§4) — needed
  before the metadata layer can be walked.

### Blockers

- None. User delegation in effect for green PRs (rebase-merge).

---

## 2026-07-30 — Metadata reference + slab index parsers (session 10)

### Done (with evidence)

- **Layer 3 spec for §5.3 metadata reference + §5.4 slab index** —
  [limnifs/spec#19](https://github.com/limnifs/spec/pull/19).
  Two new bit-level files (`38-metadata-reference.md`,
  `39-slab-index.md`) sharing the count-prefixed-locator-list pattern.
  Metadata reference pins the "unreachable metadata" invariant (at
  least one of locator_count or inline_metadata_len must be non-zero).
  Slab index pins duplicate-slab_id detection and the cross-section
  "every slab referenced by some drop" check (deferred to the
  slab-walker layer).
- **§5.3 + §5.4 parsers + locator-entries helper** —
  [limnifs/limnifs#19](https://github.com/limnifs/limnifs/pull/19).
  Run https://github.com/limnifs/limnifs/actions/runs/30505993302.
  Three new pieces:
  - `locator::parse_locator_entries(cursor, count)` — DRY helper
    for count-prefixed locator lists. Performs the pre-allocation
    DoS check and annotates inner errors with the entry index.
  - `metadata_reference::parse_metadata_reference` — handles
    external (locators), inline (embedded blob), and mixed modes.
    Default ceilings: 4 KiB per URI, 1 MiB inline blob.
  - `slab_index::parse_slab_index` — per-slab SlabId + locator
    lists with duplicate detection. DoS check on entry_count ×
    ENTRY_FIXED_LEN.
  - 21 new unit tests. Workspace test count: 95 → 116
    (15 format, 91 core, 10 cli).

### Architecture: helper pays off immediately

The `parse_locator_entries` helper was extracted because both §5.3
and §5.4 needed the same count-then-loop pattern with the same DoS
check. The extraction paid off in the same PR that introduced it:
both parsers used the helper on first attempt, no duplicated logic.
Future sections that carry locator lists (e.g., §5.5 recipients
when specified) will reuse it too.

### Manifest walk milestone

With these parsers, the manifest can be walked end-to-end through
`header → feature flags → metadata reference → slab index`. This
is the minimum chain needed to identify what slabs and drops an
image references — sufficient to start the conformance bootstrap
with a tiny valid image (header + empty flags + minimal metadata
reference + single-slab index).

### In progress / Next

- **Layer 3 spec for §5.9 history** — list of (op, timestamp,
  inputs, params) tuples. Simplest remaining section.
- **§5.9 history parser**.
- **§5.10 Merkle root construction** — needs BLAKE3 dependency.
  Once landed, `limni verify` can prove image identity, not just
  parse the header magic.
- **§5.5 crypto params, §5.6 EC params, §5.7 DMS policy, §5.8
  delta linkage** — require sub-structure specs (HPKEEnvelope,
  SignatureBundle, TreeOp).
- **02-conformance bootstrap** — declarative YAML vector format
  plus a tiny generator. The minimum-viable image (header +
  empty flags + minimal metadata reference + single-slab index)
  is now parseable; the bootstrap can land.

### Blockers

- None. User delegation in effect for green PRs (rebase-merge).

---

## 2026-07-30 — ManifestCursor refactor + drop-store parsers (session 9)

### Done (with evidence)

- **Architectural refactor: ManifestCursor** —
  [limnifs/limnifs#15](https://github.com/limnifs/limnifs/pull/15).
  Centralises bounds checks and position tracking in one type
  (`cursor::ManifestCursor<'a>`). Every parser takes
  `&mut ManifestCursor` and returns just the parsed value. The
  module split is MECE: `cursor`, `error`, `header`,
  `feature_flags` each own one concern. Adding a section is a new
  module + parser fn — no edits to existing parsers (OCP).
  Run https://github.com/limnifs/limnifs/actions/runs/30495743346
  (all three CI checks green on ubuntu + macOS).
- **Layer 3 spec for slab header (§3.2)** —
  [limnifs/spec#16](https://github.com/limnifs/spec/pull/16).
  `bit-level/30-slab-header.md` documents the 56-byte fixed prefix:
  magic `LIM1`, u16 LE `format_version`, `SlabId` (ordinal u64 LE +
  32-byte hash), u64 LE `total_length`, u8 `ec_descriptor`, u8
  `crypto_hint`. Two worked examples (plaintext slab; EC + sealed
  slab).
- **Layer 3 spec for drop record (§3.3)** —
  [limnifs/spec#17](https://github.com/limnifs/spec/pull/17).
  `bit-level/31-drop-record.md` documents the 48-byte descriptor:
  drop_id, plaintext_len, representation triple, solid_window_index,
  offset_in_window, len_in_window. Pins the slab-vs-record
  cross-field consistency rules (`aead` must be 0 in plaintext slab,
  `ec` must be 0 in no-EC slab).
- **Drop-store parsers** —
  [limnifs/limnifs#16](https://github.com/limnifs/limnifs/pull/16).
  Two new modules in `limnifs-core`:
  - `slab` — `parse_slab_header` validates magic LIM1,
    `format_version=1`, `total_length` floor and ceiling (default
    64 MiB per §3.1; overridable via `parse_slab_header_with_ceiling`).
    Rejects `ec_descriptor=0xFF` and `crypto_hint=0xFF` (extended
    sentinels, post-v1).
  - `drop_record` — `parse_drop_record` performs the cross-field
    consistency checks against the parsed slab header, the u32
    overflow check on `offset_in_window + len_in_window`, and a
    caller-overridable ceiling on `plaintext_len`.
  - 19 new unit tests (10 slab + 9 drop record), workspace test
    count 60 → 79.
  - Run https://github.com/limnifs/limnifs/actions/runs/30496254812.

### Architecture: cursor pattern pays off

The cursor refactor was the right move at the right time. The two
new parsers (`slab`, `drop_record`) landed with zero changes to the
existing parsers (`header`, `feature_flags`) — pure OCP. Each new
parser is ~100 lines plus tests; without the cursor, each would
have re-implemented bounds checking (~30 lines per parser) and
offset bookkeeping. The `&SlabHeader` parameter on
`parse_drop_record` makes the cross-field dependency explicit in
the type signature — readers can't accidentally parse a drop record
without first having parsed its containing slab.

### DoS hardening

The cursor refactor introduced an explicit pre-allocation check in
the feature flags parser: the declared `entry_count` is verified
to fit the remaining buffer BEFORE `Vec::with_capacity` is called.
Without this, a malicious manifest declaring `u32::MAX` entries
would request a multi-GB allocation and abort the reader.

### In progress / Next

- **Layer 3 spec for locator entry** (`bit-level/37-locator-entry.md`)
  — unblocks §5.3 metadata reference parser. Locator wire form is
  `scheme ":" scheme_specific_part` per [§12](https://github.com/limnifs/spec/blob/main/registries/12-locator.md);
  the bit-level file pins the length-prefix and the per-scheme body
  shape.
- **Layer 3 spec for §5.3 metadata reference** — needs locators
  first. Section carries H(metadata) + locator entries + optional
  inline blob.
- **Extend `limnifs-core` to parse §5.3 metadata reference** —
  depends on the two spec increments above.
- **Layer 3 spec for inode, Merkle B-tree node** (§4) — bigger
  chunks; needed before the metadata layer can be walked.
- **02-conformance bootstrap** — declarative YAML vector format
  plus a tiny generator that emits a header-only valid image. The
  Rust reader's existing parsers already cover this case; the
  bootstrap establishes the framework.

### Blockers

- None. User delegation in effect for green PRs (rebase-merge).

---

## 2026-07-30 — Spec Layer 3 seeded + feature flags parser (session 8)

### Done (with evidence)

- **Layer 3 spec directory seeded** —
  [limnifs/spec#14](https://github.com/limnifs/spec/pull/14).
  `bit-level/` is now a first-class layer of the spec, parallel to
  `wire-format/`, `algorithms/`, `registries/`. Established the
  documentation pattern (byte-offset table, ASCII bit-position
  diagram, validation rules, worked example, cross-references) with
  the simplest fixed-width type — the 16-byte manifest header — that
  the Rust workspace had just implemented. README updates at the top
  level and within `bit-level/` itself.
- **Layer 3 bit-level layout for §5.2 feature flags** —
  [limnifs/spec#15](https://github.com/limnifs/spec/pull/15).
  v1 wire format: `[section_version: u8][entry_count: u32 LE]
  [entry × N]` where each entry is `[flag_id: u16 LE][required: u8]`
  (3 bytes). Validation rules: buffer-overflow, duplicate-flag
  detection, required-byte constraint (only `0x00`/`0x01`; other
  values are `Corrupt`, not silently coerced).
- **Rust feature flags parser** —
  [limnifs/limnifs#13](https://github.com/limnifs/limnifs/pull/13).
  Run https://github.com/limnifs/limnifs/actions/runs/30475163840 (all
  three CI checks green on ubuntu-latest and macos-latest):
  - `FeatureFlag { flag_id, required }` and `FeatureFlags { entries }`
    types with `is_empty`, `len`, `get`, `is_required` helpers.
  - `parse_feature_flags_section(bytes, offset) ->
    Result<(FeatureFlags, usize), CoreError>` returning parsed flags
    plus bytes consumed (so callers can advance to the next section).
  - `CoreError` gains `UnsupportedFeature { feature }` for unknown
    section versions — surfaces §18 policy cleanly.
  - 11 new unit tests covering happy paths (empty, single, mixed,
    nonzero offset, all standard flags) and rejection paths
    (unknown version, short prefix, truncated entries, zero flag_id,
    bad required byte, duplicate flag_id).
  - Workspace test count: 31 → 42 (15 format, 20 core, 7 cli).

### Layer 3 pattern is now established

The shape of every future Layer 3 file is pinned by these two PRs.
Next Layer 3 files (`37-locator-entry.md`,
`38-metadata-reference.md`, `30-slab-header.md`, `31-drop-record.md`,
etc.) follow the same template: heading → total width → byte-offset
table → ASCII diagram → field semantics → validation rules →
worked example → cross-references. No more "how should I write
this?" deliberation for the next dozen files.

### In progress / Next

- **Layer 3 spec for locators** (`bit-level/37-locator-entry.md`) —
  the locator-entry wire form is `scheme ":" scheme_specific_part`
  per [§12](https://github.com/limnifs/spec/blob/main/registries/12-locator.md);
  the bit-level file needs to pin the length-prefix and the
  per-scheme body shape.
- **Layer 3 spec for metadata reference** (§5.3) — needs the
  locator-entry layout above first; section carries H(metadata)
  plus one or more locator entries plus an optional inline blob.
- **Extend `limnifs-core` to parse §5.3 metadata reference** —
  depends on the two spec increments above.
- **02-conformance bootstrap** — define the smallest valid manifest
  vector (header + empty feature flags + minimal metadata reference)
  so the Rust and Python readers have a shared target.

### Blockers

- None. User delegation in effect for green PRs (rebase-merge).

---

## 2026-07-30 — Rust workspace skeleton merged (session 7)

### Done (with evidence)

- **Three-crate Rust workspace merged via rebase-merge** —
  [limnifs/limnifs#11](https://github.com/limnifs/limnifs/pull/11).
  Run https://github.com/limnifs/limnifs/actions/runs/30474110443 (all
  three CI checks green on ubuntu-latest and macos-latest):
  - `limnifs-format` — semantic types per spec §2.2: `DropId`,
    `SlabId`, `ManifestRoot`, `Tier`, `Representation`. Distinct
    newtypes (not bare aliases). RFC 4648 base32 lowercase no-pad
    encode/decode for the `b3:<base32>` multihash display form. Magic
    constants for manifest (`LMFS`) and slab (`LIM1`) headers.
  - `limnifs-core` — manifest header parser per spec §5.1: validates
    magic, parses three independent u16 LE version fields (drop store
    / metadata / manifest), enforces the reserved-zero invariant.
    `CoreError` enum (`TooShort` / `BadMagic` / `Corrupt`) with
    human-readable `Display`.
  - `limni` — clap-based CLI exposing `limni verify <image> [--json]`
    with stable exit codes (0 success, 1 read/format, 2 usage).
  - Workspace lints: `unsafe_code = "forbid"`, `clippy::pedantic =
    "warn"`. `rust-toolchain.toml` pins stable + rustfmt + clippy +
    rust-docs.
  - Local CI matrix all green: `fmt --check`, `clippy --all-targets
    --all-features -- -W clippy::pedantic -D warnings`,
    `build --all-targets --all-features --locked`, `test --all-targets
    --all-features` (31 tests), `RUSTDOCFLAGS="-D warnings" cargo doc`.
- **Spec §5.1 clarification merged via rebase-merge** —
  [limnifs/spec#13](https://github.com/limnifs/spec/pull/13). The
  section table now states reserved = 6 bytes, reconciling the
  "first 16 bytes" opener with the field sum (4 + 2 + 2 + 2 + 6 = 16).
  Reader-side rejection rule for non-zero reserved values pinned.
  Caught while writing the Rust manifest header parser.

### A spec inconsistency found and fixed

The first integration of spec → code surfaced an internal
inconsistency in §5.1 (table summed to 14 bytes; section opener said
"first 16 bytes"). Per "spec-first" (CAMPAIGN.md non-negotiable rules),
the spec was fixed in lockstep with the code — not the other way
around. Pattern to repeat: any code that finds a spec inconsistency
gets a spec PR alongside.

### CI caught a local-only miss

Local clippy (Rust 1.94) accepted `map().unwrap_or()`; CI clippy
(Rust 1.97) flagged `clippy::map_unwrap_or`. Fix landed as a NEW
commit on the PR branch (`map_or(0u128, |d| d.as_nanos())`). Takeaway:
local Rust toolchain should track CI's, or the workspace should pin a
minimum clippy version. Worth a follow-up in `13-ci-releases`.

### In progress / Next

- **Layer 3 bit-level spec for manifest sections** — §5.2 (feature
  flags), §5.3 (metadata reference), §5.4 (slab index), §5.5 (crypto
  params), §5.6 (EC params), §5.7 (DMS policy), §5.8 (delta linkage),
  §5.9 (history), §5.10 (Merkle root). Current §5 is at the "what
  each section means" level; the bit-level layout is what the Rust
  reader needs next.
- **03-core-reader (manifest parser)** — extend `limnifs-core` to
  parse section bodies, one section per PR. Order: feature flags →
  metadata reference → slab index → Merkle root. Then 03-drop-store
  reader, then 03-overlay-resolver.
- **02-conformance (bootstrap)** — define the test vector format
  ([02-test-vector-format.md](02-conformance/02-test-vector-format.md))
  and the smallest valid manifest vector so the Rust and Python
  readers have a shared target.

### Blockers

- None. User delegation in effect for green PRs (rebase-merge).

---

## 2026-07-29 — Wire format pivot + plan updates (session 6)

### Done

- **Wire format pivot accepted** — user greenlit all 7 decisions
  after a 2024–2026 literature review (Prolly Trees, CDMT, EROFS,
  Avro, FlatBuffers, Cap'n Proto, SBE, MessagePack, Frozen2, SolFS,
  OCI Image Format, Lite2/Lite3, Apache Fury, Postcard). The pivot
  doc
  ([docs/superpowers/specs/2026-07-29-wire-format-pivot.md](../docs/superpowers/specs/2026-07-29-wire-format-pivot.md))
  records the seven decisions and the research basis. Memory entry
  `project_wire_format_pivot` persists the decisions across sessions.
- **The seven decisions** (all user-approved):
  1. **Custom wire format everywhere** — drop FlatBuffers; reject
     Avro, Cap'n Proto, SBE, MessagePack.
  2. **Deterministic Merkle B-tree** for the directory tree
     (Prolly-inspired, deterministic per §1.4).
  3. **Per-section / per-blob version byte** for schema versioning
     (no per-record vtables).
  4. **File extension `.lim`** (supersedes design doc's `.limni`).
  5. **Multi-language adapters**: spec-only OR Rust FFI/WASM wrap.
  6. **Multi-file, onion-layered spec** with bit-level detail (~40
     files in 7 layers).
  7. **Inode-granular delta ops in v0.1** (no SolFS-style partial-file
     ops; reversible via feature flag if Phase 1+ benchmarks demand).

### Plan updates this session

- `TODO.impl/CAMPAIGN.md` — non-negotiable rules now include custom
  wire format, deterministic Merkle B-tree, per-section versioning,
  `.lim` extension, multi-language adapter model. Each rule links to
  the pivot doc.
- `TODO.impl/README.md` — component map row for 01-spec updated
  (drop FlatBuffers); `.limni` → `.lim` references.
- `TODO.impl/01-spec/README.md` — full rewrite; drops FlatBuffers,
  adds custom wire format + Merkle B-tree + multi-file spec + adapter
  model.
- `TODO.impl/01-spec/01-spec-restructure-plan.md` — NEW task file
  planning the multi-file spec migration (file tree, 9-step migration
  plan, acceptance criteria, open questions).
- `TODO.impl/00-architecture/00-overview.md` — layer cake ASCII
  updated: "FlatBuffers, our schema" → "custom binary; deterministic
  Merkle B-tree for the directory".
- `docs/superpowers/specs/2026-07-29-wire-format-pivot.md` — NEW
  amendment to the original design doc. Comprehensive ADR-style
  record of all 7 decisions with rationale, impact, research basis.

### In progress / Next

- **Schema deprecation** (next PR): `limnifs/spec/schema/DEPRECATED.md`
  marks the FlatBuffers files as deprecated (not deleted, per
  never-delete rule).
- **Spec restructure step 1** (next session): seed the multi-file
  directory tree + Layer 0 files (README, how-to-read, glossary,
  conformance summary).
- **Spec restructure steps 2–9** (subsequent sessions): port content
  from current SPEC.md into the layered files; author Layer 3
  (bit-level) as new content; replace SPEC.md with a redirect when
  migration completes.

### Architectural decisions (permanent)

See `docs/superpowers/specs/2026-07-29-wire-format-pivot.md` for the
full ADR-style record. See also the project memory entry
`project_wire_format_pivot` (in
`~/.claude/projects/-Users-mulgogi-src-limnifs/memory/`).

### Blockers

- None. User delegation in effect for green PRs (rebase-merge).
- All session-6 scratch work went to `~/src/limnifs/.scratch/` per
  the workspace-local scratch rule (memory:
  `feedback_scratch_location`).

---

## 2026-07-29 — Part VII polish + initial FlatBuffers schema (session 5)

### Done (with evidence)

- **Part VII polish merged via rebase-merge** —
  [limnifs/spec#9](https://github.com/limnifs/spec/pull/9). §20
  resolved + §21 deferred now declare normative status explicitly,
  both §20 decisions framed as the reversible option, and every
  cross-reference points to a specific subsection (§3.3 DropRecord
  fields, §3.4 Solid windows, §5.7 DMS policy, §5.8 TreeOp, §14
  feature flags, §18 unknown-flag policy).
- **Initial FlatBuffers schema merged via rebase-merge** —
  [limnifs/spec#10](https://github.com/limnifs/spec/pull/10). Two
  files in `limnifs/spec/schema/`:
  - `schema/types.fbs` — semantic types per §2.2: `Hash` (generic
    32-byte BLAKE3), `DropId`, `SlabId`, `ManifestRoot`,
    `Representation`, `Tier` enum.
  - `schema/manifest.fbs` — manifest sections per §5: `MagicHeader`,
    `FeatureFlag`, `MetadataReference`, `CryptoParams`, `HPKEEnvelope`,
    `SignatureBundle`, `ECParams`, `ECOverride`, `DMSPolicy`,
    `ShareRecord`, `DeltaLinkage`, `TreeOp`, `HistoryEntry`,
    `SlabIndexEntry`, `LocatorEntry`, `Manifest` (root_type).
  - Spec-lint job ran `flatc --schema --binary --no-warnings` on both
    files in CI; both compiled in 48s. Schema is valid FlatBuffers.
- **SPEC.md main**: 1359 lines (was 1339). Schema files: 210 lines.

### Architectural improvements made on review (the "retain best code only" pass)

- **`Hash` struct introduced** — caught during drafting: FlatBuffers
  only allows fixed-size `[ubyte:N]` arrays inside structs, not
  inside tables. SPEC.md uses `[ubyte:32]` notation liberally for
  non-identity hashes (H(metadata), H(section_i), shard hashes,
  etc.). The `Hash` struct lets tables carry 32-byte hashes while
  preserving the exact-width guarantee from §2.2 — without falling
  back to variable-length `[ubyte]` vectors.
- **Semantic types throughout** — `ManifestRoot` for the merkle root
  and `base_root`, not bare `Hash`. Identity types stay distinct so
  the Rust/Python type systems can enforce them at module boundaries
  (per §2.2 "implementations MUST emit them as newtypes, not
  aliases"). A `Hash` field in a struct signals "any 32-byte BLAKE3";
  a `ManifestRoot` field signals "the image identity".
- **`image_key: [ubyte]` (variable-length)** in CryptoParams —
  deferred introducing a `Key32` struct until the pattern recurs.
  Reader-side validation enforces 32-byte length on read.
- **`inputs: [ManifestRoot]`** in HistoryEntry — vector of structs
  (FlatBuffers supports this directly); cleaner than a vector of
  fixed-size arrays.

### Workspace scratch discipline (the /tmp/ correction)

All scratch work in this session went to
`/Users/mulgogi/src/limnifs/.scratch/` (PR body files, extracted
Part VII stub, replacement prose). `/tmp/` left alone. The feedback
memory at `~/.claude/projects/-Users-mulgogi-src-limnifs/memory/`
records the rule so it persists across sessions.

### Next

- **Schema follow-ups** (in priority order):
  1. `schema/slab.fbs` — slab header, DropRecord, SlabRef, shard
     records (per §3, §16).
  2. `schema/fs.fbs` — inode, directory entries, slice map, xattrs,
     Seine per-class records (per §4).
  3. `schema/delta.fbs` — delta manifest specializations (per §5.8,
     §8.1) — or fold into manifest.fbs if the specializations are
     minimal.
- **Rust workspace + `limnifs-format` crate** — generates bindings
  from the schema files. Lands once `slab.fbs` and `fs.fbs` exist
  (the reader needs both to be useful).
- **Python bindings** — same schema, different generator. Lands
  alongside or just after the Rust crate.
- **Registries as data** (`01-feature-flag-registry`): produces the
  actual `registries/*.toml` files matching the format pinned in §9.
  Can run in parallel with the schema work.

### In progress / Blockers

- Nothing mid-flight. No blockers.

---

## 2026-07-29 — Phase 0 Track A spec self-sufficiency complete (session 4)

### Done (with evidence)

- **Part IV prose merged via rebase-merge** —
  [limnifs/spec#6](https://github.com/limnifs/spec/pull/6). §9 Registry
  format (data file shape, ID stability, "add row + regenerate
  bindings, no code change" OCP rule, codegen targets with CI diff
  gate); §10 AEAD registry (5 rows; XChaCha20-Poly1305 mandatory
  baseline); §11 Codec registry (5 rows; store + lz4 mandatory;
  determinism requirement as a conformance rule); §12 Locator scheme
  registry (6 schemes; `file:` mandatory; locator-entry wire format);
  §13 Classifier class registry (5 Seine classes; binary is fallback);
  §14 Feature-flag registry (13 v0.1 flags; ID range convention
  `0x0001–0x00FF` standard, `0x0100–0x01FF` experimental).
- **Part V prose merged via rebase-merge** —
  [limnifs/spec#7](https://github.com/limnifs/spec/pull/7). §15
  Cryptography (image key + HPKE per-recipient wrap; AEAD application
  rule pinned to `02-algorithms.md §5`; optional sigstore signature
  bundle); §16 Erasure coding (Reed-Solomon over GF(2^8) per
  `02-algorithms.md §7`; reconstruction trigger; image-level vs
  slab-level EC override semantics).
- **Part VI prose merged via rebase-merge** —
  [limnifs/spec#8](https://github.com/limnifs/spec/pull/8). §17
  Versioning policy (per-layer versions; compatibility rules; the
  "feature flags vs versions" independence rule; "IDs and field
  offsets NEVER reused" deprecation); §18 Unknown-flag policy
  (required-unknown ⇒ `UnsupportedFeature`; optional-unknown ⇒ ignore;
  per-registry behavior on unknown IDs); §19 Conformance (ten vector
  classes; Python reference reader as spec-sufficiency oracle).
- **SPEC.md main**: 1339 lines (was 1038). Eight commits in linear
  history on `limnifs/spec/main`.

### Spec v0.1 self-sufficiency: ACHIEVED

After Parts I–VI, the spec is fully self-sufficient for the
`01-format-spec-v01` acceptance criterion ("a reader can be implemented
from it alone"). A reader implementing from the spec can now:

- Decode every semantic type (`DropId`, `ManifestRoot`, `SlabRef`, etc.
  — Part I, §2.2).
- Open a manifest, verify the Merkle root, parse every section
  (Part II, §5).
- Walk a slice byte range to drops to slab extents, applying the
  "do not inflate a full slab outside recorded solid blocks" rule
  (Part III, §6).
- Resolve an overlay chain with cycle detection and depth limits
  (Part III, §7).
- Perform Delta / Flatten / Turnover / Deepen and update history
  (Part III, §8).
- Interpret every registry id (AEAD, codec, locator, classifier, flag —
  Part IV, §9–14).
- Apply wire-format crypto + EC invariants (Part V, §15–16).
- Handle versioning and unknown flags, and understand what
  conformance means (Part VI, §17–19).

The Python reference reader (`limnifs/limnifs-py`) can now be
written **from the spec only** — it doesn't need to read the Rust
implementation. That's the spec-sufficiency oracle (Part VI, §19.2).

### Architectural improvements (the "retain best code only" pass in this session)

- Caught and fixed a character-level mismatch in §5's Merkle formula
  text (Edit tool failed on a 24-line block; used Python via Bash
  for byte-exact substitution — added the trailing blank line that
  the actual file had but my old block missed).
- Pinned the **registry ID width convention** as per-registry (u8 for
  AEAD/codec/classifier, u16 for feature flags) rather than a single
  global width — matches the cardinality differences between
  registries.
- Strengthened §18.3 to include the compile-time vs runtime registry
  reader split: a generated-enum reader cannot encounter "unknown"
  rows (it fails to compile); a forward-compatible in-memory parser
  follows the per-registry rules.
- Strengthened §19.1 with ten concrete vector classes — a strict
  enumeration of what conformance vectors must cover, so the
  `02-conformance` task has a checklist.
- Cross-references tightened: §10 → `02-algorithms.md §5`; §11 →
  determinism (§1.4); §13 → `02-algorithms.md §4`; §16 →
  `02-algorithms.md §7`; §15 → `02-algorithms.md §5`. Every
  cross-reference is now an exact section pointer.

### Decisions (session 4)

- **Scratch location**: workspace-local
  `/Users/mulgogi/src/limnifs/.scratch/`, NOT `/tmp/`. Reason:
  `/tmp/` is OS-managed ephemeral scratch; project work — even
  intermediate, non-committed work — belongs in the project's
  workspace. `/tmp/` stays reserved for OS-level ephemeral use
  (lock files, sockets, mktemp outputs).
- **Spec v0.1 frozen at Part VI**: Parts I–VI cover everything a
  reader needs to decode a `.limni` image. Parts VII (§20–21
  resolved/deferred — already in good shape) and VIII (§22 worked
  examples — stubs only) are supplementary.

### Next (session 5 candidates)

- **Part VII polish** (§20–21): tighten cross-references; ensure §20
  decisions and §21 deferrals read as normative prose rather than
  stub bullets.
- **Part VIII worked examples** (§22): byte-level walks for the
  four cases (single uncompressed, delta chain depth 2, encrypted
  single-recipient, EC k=4 m=2). Stubs currently; full walks require
  matching conformance vectors.
- **Then in parallel**: [01-flatbuffers-schema] (consumes Part II §3–§5
  wire-format details + Part IV §10–§14 AEAD/codec/locator/classifier/
  feature-flag IDs) and [01-feature-flag-registry] (produces the
  actual `registries/*.toml` data files matching Part IV §9 format).

### In progress / Blockers

- Nothing mid-flight. No blockers.

---

## 2026-07-29 — Phase 0 Track A prose ramp (session 3)

### Done (with evidence)

- **Part II prose merged via rebase-merge** —
  [limnifs/spec#4](https://github.com/limnifs/spec/pull/4). §3 Drop
  store (slab format, header layout, `DropRecord` fields, solid
  windows with explicit boundaries per §20.1, optional EC shards);
  §4 Filesystem metadata (inode fields, directory entries, content
  handle + slice map, symlink/special handling, xattr namespaces,
  atime omission semantics, Seine per-class records); §5 Manifest
  (10 sections detailed — magic `LMFS`, per-layer versions, feature
  flags, metadata reference, slab index, crypto params with HPKE
  envelopes, EC params, DMS Shamir policy, delta linkage, history,
  explicit Merkle root formula).
- **Part III prose merged via rebase-merge** —
  [limnifs/spec#5](https://github.com/limnifs/spec/pull/5). §6 Two-
  level addressing (three-step resolution, `SlabRef` field order
  pinned from §2.2, range read invariants including "do not inflate a
  full slab outside recorded solid blocks"); §7 Overlay chains
  (resolution walk, format-unbounded depth with reader policy
  `overlay_max_depth` default 64, cycle detection, meromictic state
  validity); §8 Derivation operations (Delta with deterministic diff
  rule, Flatten O(metadata) zero-data-I/O with byte-identical post-
  condition, Turnover the only tier that moves bytes with cancel-safety
  and implicit exact GC, Deepen as strict representation-plane
  append with identity invariant preserved).
- **SPEC.md main**: 1038 lines (was 561). Five commits in linear
  history on `limnifs/spec/main`.

### Spec self-sufficiency status (toward `01-format-spec-v01` acceptance)

After Parts I, II, III, the spec covers identity, types, three-layer
wire format, addressing, overlay chains, and derivation operations.
A reader implementing from the spec can now:

- Decode `DropId`, `ManifestRoot`, `SlabRef`, every other semantic
  type (Part I, §2.2).
- Open a manifest, verify the Merkle root, parse every section
  (Part II, §5).
- Walk a slice byte range to drops to slab extents (Part III, §6).
- Resolve an overlay chain with cycle detection (Part III, §7).
- Perform Delta / Flatten / Turnover / Deepen and update history
  (Part III, §8).

What's still missing for full self-sufficiency:

- Part IV (registries: AEAD IDs, codec IDs, locator schemes,
  classifier classes, feature flags, registry format) — needed so a
  reader can interpret the AEAD / codec / ec / locator ids that
  appear in the wire format.
- Part V (crypto + EC references — the implementation details live
  in `05-crypto` and `07-ec`, but the spec must state the wire-
  format constraints).
- Part VI (versioning + unknown-flag + conformance — needed so a
  reader knows what to do with an unsupported flag and what
  "conformance" means).
- Parts VII–VIII polish: §20/§21 already pinned; worked examples
  (§22) need byte-level walks.

### Next

- **Part IV prose** (§9 registry format + §10–14 registry content).
  After Part IV lands, the [01-flatbuffers-schema] task can consume
  §4 field semantics, and [01-feature-flag-registry] can produce the
  registry data files.
- **Parts V–VIII prose** (V: §15–16 crypto/EC references; VI:
  §17–19 versioning + unknown-flag + conformance; VII: §20–21 polish
  with prose; VIII: §22 byte-level worked examples).
- **Then in parallel**: [01-flatbuffers-schema] (consumes §3, §4, §5
  wire-format details) and [01-feature-flag-registry] (consumes
  §9–14 registry format).

### In progress / Blockers

- Nothing mid-flight. No blockers.

---

## 2026-07-29 — Phase 0 Track A first prose (session 2)

### Done (with evidence)

- **Spec v0.1 outline merged via rebase-merge** —
  [limnifs/spec#2](https://github.com/limnifs/spec/pull/2). Before merge,
  fixed two outline issues as a NEW commit on the PR branch (not amended
  — see Decisions):
  - Added "Metadata reference" as manifest §5 item 3 (the Merkle root
    formula in `02-algorithms.md §3` commits to `H(metadata)`, so the
    manifest must carry a section recording it).
  - Made the Merkle root formula explicit
    (`H(metadata) || H(section_1) || … || H(section_9)`) and fixed a
    wrong cross-reference in §1 (was `(§5.8)` pointing to History;
    corrected to point at the Merkle root construction).
- **Spec v0.1 Part I prose merged via rebase-merge** —
  [limnifs/spec#3](https://github.com/limnifs/spec/pull/3). §1
  Foundational invariants (identity rule, image identity, representation
  plane separation, determinism) and §2 Terminology (limnologic
  vocabulary + semantic type widths) now have full normative prose.
- **SPEC.md main**: 561 lines (was 418). Three commits in linear history
  on `limnifs/spec/main`.

### Decisions resolved in v0.1 (recorded in spec §20)

- **Solid-block boundaries**: per-slab solid windows with explicit
  boundaries; cross-slab class groups deferred to a `solid-blocks-v2`
  feature flag.
- **Rename semantics**: no first-class `Rename` op in v0.1; the delta
  builder compiles detected renames to `Remove+Add`. First-class rename
  deferred to a `rename-ops` feature flag.

### Deferred to other components (spec §21)

- FastCDC parameters and minimum drop size → `04-writer-pipeline`.
- Time-lock puzzle calibration → `05-crypto` (v1 ships Shamir-only).

### Next

- Part II prose (§3–5: drop store, metadata, manifest — the three layers).
- Part III prose (§6–8: addressing, overlays, derivation operations).
- Parts IV–VIII prose (§9–22: registries, crypto/EC references,
  versioning, conformance, worked examples).
- Then in parallel: [01-flatbuffers-schema] (consumes §3, §4, §5 wire
  details) and [01-feature-flag-registry] (consumes §9–14).

### Decisions (session 2)

- **Merge strategy switched**: rebase-merge for green PRs (was squash
  in session 1). Reason: "retain best code only" — rebase preserves
  every commit's content on `main`; squash collapses them.
- **Outline polish before merge**: pre-merge fixes land as NEW commits
  on the PR branch, not amend. The PR diff shows the cleanup explicitly
  so reviewers can see the before/after.

### In progress / Blockers

- Nothing mid-flight. No blockers.

---

## 2026-07-29 — Day 0 setup (session 1)

### Done (with evidence)

- **5 org repos created** (all public):
  [limnifs/limnifs](https://github.com/limnifs/limnifs),
  [limnifs/spec](https://github.com/limnifs/spec),
  [limnifs/limnifs-py](https://github.com/limnifs/limnifs-py),
  [limnifs/limnifs-frozen2](https://github.com/limnifs/limnifs-frozen2),
  [limnifs/.github](https://github.com/limnifs/.github).
- **Each repo bootstrapped** with its first commit on `main` (the only
  main-push override granted for the campaign). Local clone layout:
  `~/src/limnifs/{repo}`.
- **`13-actions-matrix` complete** —
  [task file](13-ci-releases/13-actions-matrix.md) marked `done` with both
  halves of the acceptance evidence linked:
  - Green (empty skeleton): [limnifs/.github#1](https://github.com/limnifs/.github/pull/1) —
    run https://github.com/limnifs/.github/actions/runs/30426996117
  - Red (mutant `todo!()`): [limnifs/limnifs#2](https://github.com/limnifs/limnifs/pull/2)
    (closed unmerged) — run https://github.com/limnifs/limnifs/actions/runs/30432668400
- **Per-repo CI callers wired** — PRs
  [limnifs/limnifs#1](https://github.com/limnifs/limnifs/pull/1),
  [limnifs/spec#1](https://github.com/limnifs/spec/pull/1),
  [limnifs/limnifs-py#1](https://github.com/limnifs/limnifs-py/pull/1),
  [limnifs/limnifs-frozen2#1](https://github.com/limnifs/limnifs-frozen2/pull/1),
  all merged green; every product repo's `main` now runs the matrix on push
  and on PR.

### In progress

- Nothing mid-flight.

### Next

- Phase 0 Track A: `01-spec` — draft spec v0.1 in `limnifs/spec`, then
  FlatBuffers schema, then feature-flag registry. See
  [01-spec/README.md](01-spec/README.md) and the task files within.

### Blockers

- None. User delegation in effect for this campaign: green PRs may be merged
  by the agent via `gh pr merge --squash --delete-branch`. No tags, no
  main-push, no red merges.

### Decisions

- Local clone layout: `~/src/limnifs/{repo}` (first level matches GitHub org
  name) — user-directed.
- Day-0 first commit on `main` per repo is the only main-push exception; all
  subsequent work goes through PRs.
- Action SHAs currently pinned to major-version tags (`@v4`, `@v5`);
  SHA pinning is a tracked follow-up in `13-actions-matrix.md`.
