# LimniFS — STATUS

Living log of work sessions. Newest entry on top. Each entry: what's done
(with CI links), what's in_progress, blockers, next.

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
