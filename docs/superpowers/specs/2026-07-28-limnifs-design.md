# LimniFS — design

- **Date:** 2026-07-28
- **Status:** Draft (pending user review)
- **Scope:** A new content-addressed, compressed, immutable filesystem format and Rust implementation, designed from DwarFS/dwarfs-t ideas but not derived from their code.
- **Vision:** A small, spec-first, security-auditable filesystem image format that compresses better than tar+gzip by an order of magnitude, reads fast from any locator (file, HTTP, S3, IPFS, P2P), supports first-class deltas and layered overlays, and can serve everything from tebako packaging to IPFS-scale content distribution.

## 1. Why not a DwarFS port

We investigated `libdwarfs/`, `dwarfs-t/`, and `dwarfs-t-rs/`. Conclusions:

- **dwarfs-t is already a far fork.** We reimplemented and cleaned up many parts (folly/fbthrift removal, FlatBuffers metadata, backend-pluggable VFS in libtfs). The remaining value is *ideas*, not code.
- **GPL-3 is excluded everywhere.** Upstream DwarFS and dwarfs-t are GPL-3; `oxalica/dwarfs-enc` (writer) is GPL; `oxalica/dwarfs` (reader) is MIT/Apache and usable for legacy reading. No GPL-3 code, headers, or schema may enter LimniFS. The FlatBuffers schema is our own, written fresh.
- **DwarFS is monolithic.** Its format is so interlocked (section types, category-specific options baked into the binary layout, C++ reader/writer as the only spec) that evolving the format means rewriting the only implementation. LimniFS is spec-first: the spec and a conformance suite are separate artifacts from any implementation.

### 1.1 DwarFS drawbacks that motivate a fresh design

**Performance**

- Read amplification on cold cache: metadata and data are interleaved per-section; a partial read over HTTP still pulls more than the requested extent.
- No small-file slab packing at the storage layer: inode chunking is good, but tiny chunks still map to scattered section offsets — Taobao TFS showed <1MB files dominate real workloads and need slab packing.
- Compression is fixed at image build time. A hot image and a cold archive pay the same build cost; there is no "write fast now, compress deeper later" path.
- Single-writer, single-reader model: no deltas, no overlays, no incremental publish. Every update is a full rebuild (mkdwarfs history feature mitigates but does not compose).

**Security**

- No authenticated encryption anywhere in the format. Integrity is a checksum; confidentiality and tamper-evidence are out of scope.
- C++ attack surface: the reader is a large C++17 codebase with rich dependencies; memory safety is by discipline, not by construction.
- No signature/supply-chain story in the format itself; trust is external (HTTPS, GPG on the side).
- GPL-3 licensing alone blocks whole classes of adopters (embedded, proprietary, Apple store).

**Adoption friction**

- Build system weight (folly historically; still a heavy C++ toolchain), FUSE-centric UX, no streaming/HTTP-native access, no multi-language bindings.
- Format evolution blocked by monolith (see above).

### 1.2 The core idea worth keeping

DwarFS's real insight is **heuristic-driven heterogeneous compression**: classify files (and fragments of files) and apply different compressors per class. LimniFS keeps this and generalizes it into a *pipeline* (§6) instead of a one-shot build decision.

## 2. Goals and non-goals

**Goals**

1. Spec-first format: versioned spec + FlatBuffers schema + conformance suite as independent artifacts; multiple implementations encouraged.
2. Content-addressed chunks: BLAKE3 of *plaintext* as chunk identity; stored form (codec ∘ encrypt ∘ erasure) is metadata, not identity (§4).
3. Layered overlays and first-class deltas, with three merge tiers including metadata-only flatten (§7).
4. Staged compression pipeline: quick write → background deep compress; quick read always (§6).
5. Streaming-native and locator-agnostic: file, HTTP range, S3, IPFS, P2P behind one locator registry (§10).
6. Security by construction: AEAD registry, deterministic nonces, signed manifests, minimal Rust core, fuzzing and conformance as first-class deliverables (§11).
7. Redundancy: slab-level Reed-Solomon erasure coding; dead man's switch support in the manifest (§8).
8. License: Apache-2.0 OR MIT for everything we write. No GPL-3 anywhere in the dependency tree.

**Non-goals**

- A POSIX read-write filesystem. LimniFS images are immutable; mutation happens by building deltas.
- Writing legacy DwarFS (Thrift/Frozen2) images. We read them (§5), we never write them.
- A kernel driver in v1. FUSE + library + optional composefs-style path (§10.3).
- Replacing libtfs's multi-format VFS role. LimniFS is one format; libtfs remains the adapter host.

## 3. Naming and terminology

**LimniFS** — λίμνη (limni), Greek "lake". Acronym: **L**ayered, **I**mmutable, **M**erkle-rooted, **N**etwork **I**mage.

- CLI: `limni`. Image extension: `.limni`. Domains registered: `limnifs.org` (homepage), `limnifs.com`/`.net` (redirects); GitHub org `limnifs`; no conflicting `limni` CLI found.
- Limnology vocabulary is load-bearing, not decorative:

| Term | Meaning in LimniFS |
|---|---|
| **slice** | Unit of user content presented to the pipeline (file or file fragment) |
| **drop** | Content-addressed chunk; the atom of storage |
| **epilimnion** | Hot tier: freshly written, fast codec (LZ4), unconsolidated |
| **metalimnion** | Transition tier: background recompression in progress (thermocline = the moving boundary) |
| **hypolimnion** | Cold tier: deep-compressed (zstd/lzma/brotli per class), consolidated slabs |
| **turnover** | Merge operation that mixes layers: flatten deltas, defrag, re-encode |
| **meromictic** | An overlay chain that is never flattened (valid long-term state) |
| **limn (verb)** | To sketch/outline an image — the CLI verb for building: `limni limn` |

## 4. Identity rule (foundational invariant)

> **Identity is BLAKE3 of plaintext.** A drop's name is `b3:<base32>` of its *decompressed, decrypted* bytes. Compression codec, encryption, and erasure coding are *representations* recorded in metadata; the same drop may exist in several representations across tiers and locators without changing identity.

Consequences:

- Deduplication is representation-independent: a slice re-compressed from epilimnion to hypolimnion still addresses identically — merges and overlays never rewrite references.
- Integrity verification is end-to-end: decrypt/decompress → hash → compare with the addressed name. AEAD tags are a fast-fail; the BLAKE3 name is the proof.
- Interop: drop names are multihash-compatible, so CAR import/export and IPFS placement are mechanical (§10.2).

## 5. Format structure: three layers

A `.limni` image is three cleanly separated layers, each independently versioned via feature flags:

1. **Drop store** — the bulk data: drops grouped into **slabs** (large contiguous objects, target 4–64 MiB). Two-level addressing: slice → drop → (slab, offset, length, representation). Slab packing is the Taobao TFS lesson: small drops must never map to scattered storage offsets.
2. **Filesystem metadata** — our own FlatBuffers schema: inodes, directory tree, xattrs, slice→drop maps, per-class compression records, tier placement. Immutable once written.
3. **Manifest** — the small, signed head of the image: schema versions, feature flags, drop-store table (slab index), crypto parameters, EC parameters, delta linkage (`base_root` if this is a delta), policy (dead man's switch, escrow), and the Merkle root. **The signed Merkle root is the image identity.**

A manifest may live detached from its drop store (URL/URI references in the locator registry), which is what makes cloud assembly and IPFS pinning the same operation as local mounting.

### 5.1 Legacy reading (never writing)

LimniFS ships a `limnifs-frozen2` read adapter, using clean-room code plus the MIT/Apache `oxalica/dwarfs` reader where useful, to mount DwarFS v2 (Thrift/Frozen2) images. Purpose: tebako's installed base and one-way migration (`limni import-dwarfs` re-encodes into `.limni`). The adapter is a separate crate so the GPL-free core never links questionable code paths, and the writer for Frozen2 is explicitly never built.

## 6. Compression pipeline (staged, LSM-style)

The pipeline is the design's answer to "different algorithms for different file kinds":

- **Ingest (epilimnion):** slices are chunked (content-defined chunking, FastCDC, 64 KiB–1 MiB target), hashed, classified, and written immediately with LZ4 (or stored). Write latency ≈ memcpy + hash. This is the "quick write" requirement.
- **Classification:** a scanner (the *seine* pass) labels drops by class — text/code, binary/executable, already-compressed, media, sparse — using entropy and magic heuristics. Classification is recorded per drop class, not baked into layout.
- **Background deepening (metalimnion → hypolimnion):** a compactor re-encodes drops class-appropriately (zstd default; lzma/brotli for text-heavy cold classes; store for incompressible), repacks slabs, and emits a *new representation* — identity unchanged, references untouched. Triggered by policy: idle, size threshold, age, or explicit `limni deepen`.
- **Read path:** always representation-aware and tier-agnostic; reads never wait for deepening. Range reads map slice → drop → slab extent with no full-slab inflation except for compressed solid blocks, whose boundaries are recorded in metadata.

This gives DwarFS-class ratios on cold images with tar-class write latency on hot ones — the thing DwarFS's one-shot build cannot do.

## 7. Deltas and overlays (first-class)

A **delta image** is a normal image whose manifest carries `base_root` plus tree operations (add/remove/replace/rename at inode granularity) and drop references into the base. Semantics:

- **Tier 1 — read-time overlay:** stack manifests; resolution walks the chain (meromictic state is legal forever). Chain depth bounded by policy; per-read cost is O(depth) metadata, not data copying.
- **Tier 2 — metadata-only flatten:** merge N manifests into one composite manifest; drops are *not* re-encoded, only re-referenced. O(metadata), seconds even for large trees. Produces a flat image that still shares drops with its bases.
- **Tier 3 — turnover (full re-encode defrag):** repack drops into fresh slabs, re-run deepening, garbage-collect unreferenced drops. Produces a standalone image with no external references.

`limni merge --flatten` (tier 2) is the default for "fold this patch into the main image"; turnover is the periodic hygiene operation. IPFS-scale usage: deltas are themselves pinnable objects; overlay chains form a version DAG with Merkle roots as edges.

## 8. Redundancy and the dead man's switch

- **Erasure coding:** optional per-slab Reed-Solomon (k+m) recorded in the manifest. Representation-level, identity-neutral (§4). Targets: IPFS/P2P where chunks vanish, and archival durability without full replication.
- **Dead man's switch:** the manifest policy section may carry either (a) a **time-lock puzzle** (iterated squaring; no trusted party) whose solution unwraps the image key after a chosen wall-clock horizon, or (b) **Shamir escrow**: key split k-of-n across named custodians with release conditions. Both are metadata-only; the core reader ignores them unless asked (`limni dms status/solve/collect`).

## 9. Cryptography

AEAD algorithm registry (1-byte tag in manifest crypto params):

| ID | Algorithm | Role |
|---|---|---|
| 0x01 | XChaCha20-Poly1305 | **Mandatory baseline** — fast everywhere, no hardware assumptions |
| 0x02 | AES-128-OCB | Fast path with AES-NI; single-pass. Patents abandoned by Rogaway (2021-02-27), RFC 7253 — legally clean |
| 0x03 | AES-256-GCM | Compliance/interop option |
| 0x04 | Ascon-128a | NIST lightweight winner; embedded/MCU readers |

Rules:

- **Deterministic nonces:** `nonce = HKDF(image_key, slab_id ‖ drop_index)` — no nonce state to manage, misuse-resistant by construction for immutable data.
- **Associated data:** `manifest_root ‖ slab_id ‖ drop_index` — a drop cannot be transplanted between images, slabs, or positions without failing authentication.
- **Keys:** image key wrapped per-recipient (X25519 HPKE) in the manifest; unsigned manifests are plain integrity mode, signed manifests carry sigstore-compatible signatures.
- Everything encrypted is also content-addressed by *plaintext* hash (§4), so dedup works across recipients sharing an image key.

## 10. Distribution: locators, streaming, kernel path

### 10.1 Locator registry

Manifests and slabs are referenced through a locator registry: `file:`, `http(s):` (range requests), `s3:`, `ipfs:`, `limni-p2p:`. A manifest can name several locators per slab (mirror list) with per-locator priority. Readers race mirrors; writers push slabs to whichever locators policy selects. Cloud **sharding and assembly** = manifest referencing slabs across N buckets/regions; **streaming-native** = read-ahead over HTTP ranges with the slab index as the seek map, no full download ever required.

### 10.2 IPFS scale

Drop names are multihash-compatible (§4); `limni export-car` / `import-car` move images to/from IPLD CAR files. Overlay chains pin as DAGs. Erasure-coded slabs survive node churn.

### 10.3 Kernel path (optional)

Linux readers may use a composefs-style path: metadata via EROFS loop mount, drops served from a content-addressed local cache filled by the locator layer. FUSE remains the portable default.

## 11. Security model

- **Language:** Rust, `unsafe` only at vetted FFI boundaries (none planned for core). Minimal `limnifs-core` reader crate with everything else (locators, writers, FUSE, P2P) as separate crates — the Redox TFS lesson: one small audited core, not a bundle of novel ideas.
- **Supply chain:** signed manifests (sigstore), reproducible builds, SBOM per release.
- **Verification program:** differential fuzzing of the reader against a Python reference reader; malicious-image corpus (truncated slabs, overlapping extents, cycle attempts in overlay chains, nonce/AD confusion); the conformance suite is the acceptance gate for any third-party implementation.
- **Threat model stated in spec:** we defend against malicious *images* and malicious *locators*; compromised readers and key theft are out of scope (DMS/escrow mitigates operator disappearance, not endpoint compromise).

## 12. Audience and resulting feature requirements

Primary audiences, and what each forces into the design:

- **Software packaging (tebako, first):** single-file executables, fast cold start, mmap-friendly reader, multi-image attach, legacy Frozen2 import. → drives the minimal core and the FUSE/library split.
- **CI/build artifact distribution:** content-addressed dedup across builds, HTTP range streaming, deltas between successive builds. → drives tier-2 flatten and locator priorities.
- **Container-adjacent images:** composefs-style kernel path, signed manifests. → drives §10.3 and sigstore.
- **Archival/compliance:** erasure coding, dead man's switch, escrow, cold-tier ratio. → drives §8.
- **IPFS/decentralized distribution:** multihash identity, CAR interop, P2P locator. → drives §10.2.

## 13. Prior art: what we take and what we avoid

| System | Take | Avoid |
|---|---|---|
| DwarFS / dwarfs-t | Heuristic per-class compression; fragment packing; dedup | Monolithic format+impl; GPL; no crypto; build-time-only decisions |
| Taobao TFS | Slab packing of small files into big objects; two-level addressing | Custom block-layer assumptions; master-server architecture (we are a format, not a service) |
| Redox TFS | Rust modularity ambition | Bundling too many novel ideas so nothing finished — we ship a walking skeleton first |
| composefs | Kernel-native verified mounting of content-addressed images | EROFS-only metadata coupling |
| IPFS/IPLD | Multihash identity, CAR transport | Linked-data overhead per block; we batch into slabs |
| git / OCI | Delta chains, content addressing, registry UX | Per-object granularity (too fine); we address slabs for storage and drops for identity |

## 14. Crate boundaries

**Repos (GitHub org `limnifs`):** separation exists only where load-bearing —
independent spec versioning, reader independence, license boundary.

- `limnifs/limnifs` — Rust workspace: all crates below plus `limni` and `limnifs-fuse`; `TODO.impl/` and docs migrate here.
- `limnifs/spec` — `SPEC.md`, FlatBuffers schema, registries, conformance vectors + harness; independently tagged.
- `limnifs/limnifs-py` — Python reference reader (written from spec only).
- `limnifs/limnifs-frozen2` — legacy adapter (license-scan boundary).
- `limnifs/.github` — org-level reusable CI workflows.

**Crates (in `limnifs/limnifs` unless noted):**

- `limnifs-core` — format reader: manifest parse, drop store, overlay resolution. No networking, no FUSE, no-std-adjacent.
- `limnifs-format` — FlatBuffers schema + builders (shared by reader/writer); generated from `limnifs/spec` at a pinned tag.
- `limnifs-write` — ingest, classification, deepening, delta builder, merge/turnover.
- `limnifs-crypto` — AEAD registry, key wrapping, DMS primitives.
- `limnifs-locator-{file,http,s3,ipfs}` — locator plugins behind one trait.
- `limnifs-frozen2` — legacy DwarFS read-only adapter (own repo).
- `limnifs-fuse`, `limni` (CLI), plus `spec/` and `conformance/` as sibling artifacts (own repo), not crates.

## 15. Phased plan

- **Phase 0 — Spec and skeleton.** Spec v0.1, FlatBuffers schema, conformance suite seed, `limnifs-core` reading trivial uncompressed images. Exit: two independent readers (Rust + Python reference) pass conformance.
- **Phase 1 — Tebako packaging.** Writer, LZ4/zstd tiers, FUSE mount, Frozen2 import, `limni` CLI; tebako press/mount path consuming `.limni`. Exit: tebako packages and runs from LimniFS images with parity tests vs dwarfs-t images.
- **Phase 2 — Cloud and streaming.** HTTP/S3 locators, range streaming, deltas + tier-2 flatten, signed manifests. Exit: CI-artifact use case live over plain HTTPS.
- **Phase 3 — Depth.** Erasure coding, DMS/escrow, IPFS locator + CAR interop, P2P, composefs path. Each lands behind a feature flag so the core stays small.

## 16. Open questions

1. Solid-block boundaries for hypolimnion text classes: per-slab solid windows vs. per-class solid groups — decide with benchmarks in Phase 1.
2. FastCDC parameters and minimum drop size under slab packing (dedup ratio vs. index size).
3. Time-lock puzzle calibration story (hardware drift between seal and solve) — likely keep DMS v1 Shamir-only.
4. Whether overlay resolution admits renames as first-class ops or compiles them to remove+add at build time (writer simplicity vs. delta size).
