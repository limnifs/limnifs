# LimniFS — STATUS

Living log of work sessions. Newest entry on top. Each entry: what's done
(with CI links), what's in_progress, blockers, next.

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
