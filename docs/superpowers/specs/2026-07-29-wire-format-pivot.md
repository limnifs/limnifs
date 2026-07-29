# LimniFS — wire format pivot

- **Date:** 2026-07-29
- **Status:** Accepted (user-approved)
- **Supersedes:** parts of [2026-07-28-limnifs-design.md](2026-07-28-limnifs-design.md) §5 (Filesystem metadata — "our own FlatBuffers schema")
- **Scope:** Foundational wire format decisions for LimniFS v0.1 onward.

## TL;DR

After a literature review of filesystem and serialization research
(2024–2026), LimniFS drops FlatBuffers (and rejects Avro, Cap'n Proto,
SBE, MessagePack) in favor of a **custom wire format owned by LimniFS**.
The format uses a **deterministic Merkle B-tree** for the directory
tree, **per-section version bytes** for schema evolution, the **`.lim`
file extension**, and a **multi-file, onion-layered spec** with bit-
level detail. Multi-language adapters (Ruby, TypeScript, Python) choose
between **spec-only implementation** (true spec-first oracle) and
**Rust FFI/WASM wrap** (fast to ship).

The seven decisions are below, each with rationale and impact.

## Decisions

### D1 — Custom wire format everywhere (drop FlatBuffers; reject Avro / Cap'n Proto / SBE / MessagePack)

**Decision:** LimniFS specifies and owns its wire format end-to-end.
The drop store (§3), metadata (§4), and manifest (§5) are all custom
binary formats specified in SPEC.md. No FlatBuffers, no Avro, no
Cap'n Proto, no SBE, no MessagePack. Schema source = SPEC.md;
codegen derives from Rust types via `serde`; no external IDL.

**Rationale:**

- **Spec-first oracle.** The Python reference reader must implement
  the format from SPEC.md alone. Avro/FlatBuffers libraries break this
  (the oracle tests the library, not the spec); implementing Avro/
  FlatBuffers from spec is the same effort as implementing custom.
- **Byte-addressability.** HTTP range streaming, mmap, the composefs-
  style kernel path all need fixed offsets. FlatBuffers' vtable and
  Cap'n Proto's pointer indirection break this. Frozen2-class custom
  preserves it.
- **No vtable overhead.** FlatBuffers tables carry ~8 B per access of
  vtable overhead. For metadata with millions of inodes, that is MBs
  of overhead and indirects every read.
- **No external dependencies.** No Google (FlatBuffers), no Facebook
  (Frozen2/folly), no Cap'n Proto LLC. LimniFS owns the format
  forever.
- **Smallest audit surface.** Custom serializer/deserializer is small
  Rust code, fully auditable. FlatBuffers/Frozen2 runtimes are large.
- **Determinism.** Custom format gives us structural control over
  determinism (§1.4). External libraries may be deterministic in
  practice but not by guarantee.
- **Spec-first purity.** SPEC.md becomes the wire format itself — no
  translation layer through `.fbs` or `.capnp` or `.avsc`.

**Research basis:** literature review covered FlatBuffers, Cap'n Proto,
SBE, MessagePack, Avro, Fury, Lite2/Lite3, Postcard, Frozen2, EROFS,
Prolly Trees, CDMT, SolFS, OCI Image Format. None unambiguously beat
custom for LimniFS's combined requirements (spec-first oracle + byte-
addressable + mmap-friendly + multi-language adapter + no external
deps).

**Impact:**

- `schema/*.fbs` files deprecated (kept per never-delete rule; see
  `limnifs/spec/schema/DEPRECATED.md`).
- `01-flatbuffers-schema` task file deprecated; replaced by
  `01-wire-format` task.
- SPEC.md restructured into multi-file (see D6).
- `limnifs-format` Rust crate becomes the custom serializer/deserializer
  (no flatc-generated bindings).

### D2 — Deterministic Merkle B-tree for the directory tree

**Decision:** The metadata layer's directory tree is a deterministic
Merkle B-tree. Each node is content-addressed by `BLAKE3(node_bytes)`.
Split decisions are deterministic (split when node has N entries; N
fixed by spec).

**Rationale:**

- **Content-addressed structural sharing.** A delta touching one file
  writes only `O(log N)` new nodes (the path from leaf to root).
  Unchanged subtrees are referenced, not copied. Critical for delta
  chain efficiency (§7–§8).
- **Multihash-compatible.** Node hashes are BLAKE3-256; the display
  form `b3:<base32>` is multihash-compatible. Maps directly to IPFS/
  IPLD and CAR interop (§10.2).
- **Prolly Tree lessons, but deterministic.** Prolly Trees (Dolt,
  production) use CDC for node boundaries — probabilistic, conflicts
  with §1.4 determinism. The deterministic variant (split at N
  entries) gives Prolly's benefits (content addressing, structural
  sharing, easy diffing) without probabilistic behavior.
- **CDMT alignment.** Content-Defined Merkle Trees (Nakamura 2021)
  solve the chunk-shift problem; the deterministic Merkle B-tree is
  the ordered-tree analog. Stable node boundaries across versions.

**Impact:**

- §4 (Filesystem metadata) gains a new subsection: directory tree as
  Merkle B-tree (replaces "directory representation deferred to schema
  task").
- New section in bit-level docs: node layout, split rules, hash
  computation.
- Delta construction (§8.1) walks the B-tree to compute minimal node
  changes; the writer pipeline outputs new nodes via `Sink.put_node`.

### D3 — Per-section / per-blob version byte (no per-record vtable)

**Decision:** Schema versioning lives at the section level (manifest)
or blob level (metadata), not per record. Within a version, every
record is fixed-width and order-pinned. Cross-version: forward
compatible (new reader reads old); backward explicit-reject (old
reader rejects new with `UnsupportedVersion`).

**Rationale:**

- **No per-record overhead.** FlatBuffers vtables cost ~8 B per table
  access; for millions of inodes that is MBs of overhead. Per-blob
  version byte is amortized across all records in the blob.
- **EROFS principle.** EROFS metadata is "directly accessible from
  block devices without decoding/deserialization" — versioning checked
  once at open time, then every read is direct offset access.
- **Simpler than Avro.** Avro's schema evolution has complex backward/
  forward/full compatibility rules. LimniFS already has feature flags
  (§14) + per-layer versions (§5.1) + "no field ever repurposed,
  deprecation = tombstone" (§17.4). Avro's model is redundant.
- **Deterministic.** Within a version, every byte is determined by
  the spec. No vtable layout choices to worry about.

**Impact:**

- §5.1 (MagicHeader) keeps `drop_store_version`, `metadata_version`,
  `manifest_version`.
- §4 (metadata) gains a `metadata_format_version: u16` at blob offset
  0.
- §3.2 (SlabHeader) keeps `format_version: u16`.
- §17 (Versioning policy) tightens: backward-incompatible changes bump
  the relevant version; forward-compatible additions may stay within a
  version (at the cost of new fields appended at section end).

### D4 — File extension `.lim`

**Decision:** Every LimniFS image file uses the `.lim` extension. The
design doc's `.limni` is superseded.

**Rationale:**

- **Shorter and more distinctive.** `.lim` is three characters, easy
  to type, easy to remember. Less generic-sounding than `.limni` (which
  is just the project name). Distinctive enough that file(1) and MIME
  registries can adopt it without conflict.
- **No standard conflicts.** `.lim` is not in widespread use as a file
  extension (no major format claims it).
- **Magic bytes still identify at byte level.** Files start with
  `LMFS` (manifest) or `LIM1` (slab), so byte-level identification
  works regardless of extension.

**Impact:**

- SPEC.md §5.1 (MagicHeader) gets a note: "Files using this format
  SHOULD use the `.lim` file extension."
- Documentation updated everywhere from `.limni` to `.lim`.
- The CLI (`limni`) is unaffected (the CLI name comes from the Greek
  word, not the extension).

### D5 — Multi-language adapter model: spec-only OR Rust FFI/WASM wrap

**Decision:** LimniFS supports two adapter implementation paths:

1. **Spec-only.** Adapter authors read SPEC.md and implement the format
   in their target language (Ruby, TypeScript, Python, future). No
   LimniFS-supplied runtime; the spec is the contract.
2. **Rust FFI / WASM wrap.** Adapter authors wrap the Rust
   `limnifs-core` crate via FFI (Ruby via `ffi` gem, C extension) or
   WASM (TypeScript via `wasm-bindgen` output).

Both paths are documented and supported. Conformance vectors are
black-box (architecture §I12) and work for both.

**Rationale:**

- **Spec-first oracle preserved.** The Python reference reader is the
  canonical spec-only implementation. It verifies the spec is
  unambiguous.
- **Adapter flexibility.** Ambitious adapters (full standalone reader
  in Ruby or TS) choose spec-only. Pragmatic adapters (CLI tool that
  needs to read `.lim` files) wrap Rust.
- **No Avro/FlatBuffers library needed.** The whole point of D1 is
  no external serialization deps; this holds for adapters too.

**Impact:**

- New section in SPEC.md: `multi-language/60-adapter-paths.md` (path
  overview), `multi-language/61-ruby-adapter.md`,
  `multi-language/62-typescript-adapter.md`,
  `multi-language/63-python-reference.md`.
- `limnifs-core` crate exposes a C-ABI surface for FFI consumers.
- `limnifs-core` crate compiles to `wasm32-unknown-unknown` target for
  WASM consumers (CI verifies this).
- Conformance harness (component 02) treats all adapters as black boxes
  via stdin/stdout protocol.

### D6 — Multi-file, onion-layered spec with bit-level detail

**Decision:** SPEC.md (the current single 1359-line file) is
restructured into a multi-file document organized in onion layers.
Every fixed-width type is specified down to bit position.

**Onion layers (information exposure):**

- **Layer 0 (orientation):** README.md, how-to-read, glossary,
  conformance summary. A reader who only reads Layer 0 understands
  what LimniFS is and how to use it.
- **Layer 1 (concepts):** overview, identity model, three layers,
  representations, versioning, distribution, derivations. A reader
  who reads Layer 1 understands the system's design.
- **Layer 2 (wire format):** section-level descriptions of drop store,
  metadata, manifest, locators, Merkle B-tree. A reader who reads
  Layer 2 can navigate a `.lim` file at the section level.
- **Layer 3 (bit-level):** byte and bit layouts for every fixed-width
  type. A reader who reads Layer 3 can implement a parser.
- **Layer 4 (algorithms):** resolution, build, deepen, delta, flatten,
  turnover, verify. A reader who reads Layer 4 can implement a full
  reader/writer.
- **Layer 5 (conformance):** vectors, test format, reference reader
  contract. A reader who reads Layer 5 can run conformance.
- **Layer 6 (registries + multi-language + appendices):** reference
  data and adapter guides.

**File tree (proposed):**

```
limnifs/spec/
├── README.md                          # L0 entry point
├── 00-how-to-read.md                  # L0 reading guide
├── 01-glossary.md                     # L0 terms
├── 02-conformance-summary.md          # L0 conformance overview
├── concepts/                          # L1
│   ├── 10-overview.md
│   ├── 11-identity.md
│   ├── 12-layers.md
│   ├── 13-representations.md
│   ├── 14-versioning.md
│   ├── 15-distribution.md
│   └── 16-derivations.md
├── wire-format/                       # L2
│   ├── 20-file-layout.md
│   ├── 21-drop-store.md
│   ├── 22-metadata.md
│   ├── 23-manifest.md
│   ├── 24-locators.md
│   └── 25-merkle-btree.md
├── bit-level/                         # L3
│   ├── 30-slab-header.md
│   ├── 31-drop-record.md
│   ├── 32-representation.md
│   ├── 33-inode.md
│   ├── 34-merkle-btree-node.md
│   ├── 35-manifest-header.md
│   ├── 36-manifest-sections.md
│   ├── 37-locator-entry.md
│   ├── 38-history-entry.md
│   └── 39-tree-op.md
├── algorithms/                        # L4
│   ├── 40-read-path.md
│   ├── 41-build-path.md
│   ├── 42-deepen.md
│   ├── 43-delta-build.md
│   ├── 44-flatten.md
│   ├── 45-turnover.md
│   ├── 46-verify.md
│   └── 47-merkle-root.md
├── conformance/                       # L5
│   ├── 50-vectors.md
│   ├── 51-test-format.md
│   └── 52-reference-reader.md
├── registries/                        # L6 data
│   ├── README.md
│   ├── aead.toml
│   ├── codec.toml
│   ├── locator.toml
│   ├── classifier.toml
│   └── feature-flags.toml
├── multi-language/                    # L6 adapters
│   ├── 60-adapter-paths.md
│   ├── 61-ruby-adapter.md
│   ├── 62-typescript-adapter.md
│   └── 63-python-reference.md
├── appendices/                        # L6 reference
│   ├── A-references.md
│   ├── B-change-log.md
│   ├── C-decision-records.md
│   └── D-open-questions.md
├── schema/                            # DEPRECATED (FlatBuffers; kept per never-delete rule)
│   ├── DEPRECATED.md
│   ├── types.fbs
│   └── manifest.fbs
└── SPEC.md                            # Redirect to README.md (backward compat)
```

**Rationale:**

- **Educational.** Onion layers let readers enter at the depth they
  need. A user wanting to mount a `.lim` image reads Layer 0. An
  adapter author reads Layer 3. A conformance engineer reads Layer 5.
- **Navigable.** Each file is focused (1 topic, ~200–500 lines max).
  Cross-references link layers.
- **Bit-level detail.** Layer 3 is uncompromising: every fixed-width
  type has a byte-offset table and a bit-position diagram. No
  ambiguity.
- **MECE.** Each topic lives in exactly one file. No overlap.
- **DRY.** Each fact stated once, referenced elsewhere.

**Impact:**

- New task: `TODO.impl/01-spec/01-spec-restructure-plan.md` plans the
  file-by-file migration from the current single-file SPEC.md to the
  multi-file structure.
- The current SPEC.md (1359 lines) becomes the source material; its
  content is split across Layer 1–3 files.
- The schema/*.fbs files stay (deprecated) and the new bit-level layer
  references them as historical context.

### D7 — Stay with inode-granular delta ops in v0.1 (no SolFS-style partial-file ops)

**Decision:** v0.1 delta operations are `Add / Remove / Replace` at
inode granularity (per existing §5.8, §20.2). No partial-file `Update`
op, no first-class `Rename` op. Partial-file updates produce new drops
via FastCDC; the inode's slice map references them.

**Rationale:**

- **LimniFS is distribution-focused, not mobile-backup-focused.**
  SolFS (ATC '25) targets mobile cloud backup (write-heavy, partial-
  sync updates); its finer-granular ops suit that niche. LimniFS
  targets distribution (read-heavy, whole-file content); inode-
  granularity is sufficient.
- **Writer pipeline already chunks at the drop level.** A partial file
  update creates new drops via FastCDC; the inode's slice map updates
  to reference them. No need for op-level partial updates.
- **Reversible decision.** If Phase 1+ benchmarks show delta bloat for
  write-heavy workloads, we can add `Update(path, range, drops)` as a
  feature flag (registry-gated, OCP-pure per §9). Not v0.1.

**Impact:**

- §8.1 (Delta) unchanged in scope; prose tightened to frame flatten +
  turnover as "squeeze" operations (SolFS terminology, applicable).
- §20.2 (Rename semantics) unchanged.
- Future spec amendment may add partial-file ops behind a feature flag.

## What does NOT change

- **Identity rule** (§1.1): `DropId = BLAKE3(plaintext)`.
- **Image identity** (§1.2): `ManifestRoot` = Merkle root.
- **Representation plane separation** (§1.3).
- **Determinism** (§1.4).
- **Registries as data** (§9–§14) — these are TOML data files, not
  affected by wire format choice.
- **Locator racing** (§12, architecture §I9).
- **Three-tier merge** (§7–§8) — overlay, flatten, turnover.
- **Conformance model** (§19) — vector classes, Python oracle.
- **Frozen2 legacy read adapter** (component 09) — separate repo,
  reads existing Frozen2 images, never writes.

## Implementation order (next sessions)

1. **Spec restructure plan** (this PR series) — TODO.impl/01-spec/
   01-spec-restructure-plan.md.
2. **Schema deprecation** — `limnifs/spec/schema/DEPRECATED.md`.
3. **Spec file tree seeding** — create the directory structure and
   Layer 0 files (README, how-to-read, glossary, conformance summary).
4. **Layer 1 (concepts) migration** — port from current SPEC.md
   Parts I, III.
5. **Layer 2 (wire format) migration** — port from current SPEC.md
   Parts II, IV, V.
6. **Layer 3 (bit-level) authoring** — new content, down to bit
   position.
7. **Layer 4 (algorithms) migration** — port from current SPEC.md
   §6, §8.
8. **Layer 5 (conformance) migration** — port from §19.
9. **Layer 6 (registries + multi-language + appendices)** — port from
   §10–§14, §A–§B; new multi-language content.
10. **Old SPEC.md** — replace content with redirect to README.md
    (backward compat for existing links).

## Research sources

(See the literature review in session 6 chat; key citations below.)

- Prolly Trees — DoltHub blog (July 2025); Rawat 2024 academic paper.
- CDMT — Nakamura 2021 (arXiv 2104.02158).
- EROFS — kernel docs; Linux 6.17 metadata compression (2025).
- Avro — Apache Avro 1.11.1 specification.
- FlatBuffers / Cap'n Proto / SBE — Kenton Varda comparison (2014).
- SolFS — USENIX ATC '25 (Pan et al.).
- OCI Image Format — opencontainers/image-spec.
- DwarFS Frozen2 — mhx/dwarfs discussions.

## Status

Accepted by user 2026-07-29. Implementation begins session 7+.
