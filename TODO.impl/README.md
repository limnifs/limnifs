# LimniFS — implementation work breakdown

Root of the LimniFS implementation tree. Design rationale lives in
[`docs/superpowers/specs/2026-07-28-limnifs-design.md`](../docs/superpowers/specs/2026-07-28-limnifs-design.md)
("the design doc"); this tree is the actionable decomposition. The design doc
is the **why**; this tree is the **what**; each component README is the
**contract**; each task file is the **unit of work**.

## 1. Governing principles (non-negotiable)

| Principle | How it is enforced here |
|---|---|
| **OCP (open/closed)** | Every variation point is a *registry*, never a switch statement: AEAD registry (design §9), codec registry, locator registry (§10.1), classifier registry, feature-flag registry (§5). Adding an algorithm/locator/classifier = registering a new entry; existing code does not change. |
| **Encapsulation** | Three format layers (drop store / metadata / manifest) are separately versioned and separately parsed; no layer reaches into another's bytes. Crates expose traits, not structs, across boundaries. |
| **Modularity** | One crate per component (design §14). `limnifs-core` stays minimal and no-std-adjacent; everything else is a plugin. |
| **Extensibility** | Feature flags gate every post-v1 capability; unknown flags degrade to clean errors, never to misreads. |
| **MECE** | The component map (§3 below) partitions the system with no overlap and no gap. If a task doesn't fit exactly one component, the map is wrong — fix the map, not the task. |
| **SSOT** | The FlatBuffers schema (`01-spec`) is the single source of truth for all wire formats; this tree is the SSOT for work state; the design doc is the SSOT for decisions. Nothing is specified twice. |
| **DRY** | All language bindings, test vectors, and doc tables are *generated* from the schema and registries, never hand-copied. |
| **Model-driven** | Code is generated from models (schema, registries, test-vector definitions). Hand-written code implements behavior only. |
| **Semantically-driven** | Semantic newtypes everywhere (`DropId`, `SlabId`, `ManifestRoot`, `Tier`); no bare `u64`/`[u8; 32]` crosses a module boundary. Vocabulary follows design §3 (drop, slab, epilimnion, turnover…). |
| **DRY errors** | One error enum per crate, `thiserror`-style, no panics on untrusted input. Error taxonomy specified in `00-architecture/01-interfaces.md`. |
| **Performance** | Zero-copy reads (bytes are borrowed, never re-buffered); mmap for local slabs; range streaming for remote; bounded memory regardless of image size; no full-slab inflation outside recorded solid blocks. |
| **No shims, no stubs** | Every merged task is complete and functional. Forbidden: `todo!()`/`unimplemented!()` in merged code, no-op trait impls, "placeholder" returns, disabled tests, feature flags hiding unfinished paths. Partial work lives on branches; it does not land. If a task is too big to finish whole, split the task file first (fix the map, not the code). |
| **GitHub Actions proof** | Nothing is "done" by claim. Every task's acceptance criteria run in GitHub Actions (matrix: linux + macOS, stable Rust), and the task file records the green run. |
| **Cleanliness gates** | `clippy::pedantic` clean, no `unsafe` outside vetted FFI, public API doc coverage 100%, conformance suite green before any merge. |

## 2. Global invariants

1. **Identity rule** (design §4): `DropId = BLAKE3(plaintext)`. Codec, encryption, and erasure coding are *representations*, recorded as metadata, never part of identity.
2. **Manifest root = image identity** (design §5): the signed Merkle root names the image; everything else is reachable from it.
3. **Two-level addressing**: slice → drop → (slab, offset, len, representation). Small drops never map to scattered storage (Taobao TFS lesson).
4. **Dependency direction is acyclic**: `01-spec` → `03-core-reader` → (`05-crypto`, `07-erasure-coding`, `08-locators`) → (`04-writer-pipeline`, `06-deltas-overlays`) → (`10-cli`, `11-mount`, `12-tebako-integration`). `02-conformance` may depend on everything; nothing depends on it. `09-legacy-frozen2` depends only on `01-spec` + `03-core-reader` traits. `13-ci-releases` orchestrates everything and owns no code. `00-architecture` is documentation: it *specifies* the interaction points every component must honor; where a component README and `00-architecture` disagree, `00-architecture` wins and the README is fixed (SSOT for interfaces).
5. **No GPL-3 anywhere** (design §1): license scan is part of CI (owned by 13).

## 3. Component map (MECE)

**Repository topology (GitHub org: [`limnifs`](https://github.com/limnifs))** —
repos exist where separation is load-bearing, crates share a repo where
atomic cross-crate PRs matter:

| Repo | Contains | Why separate (or not) |
|---|---|---|
| `limnifs/limnifs` | Rust workspace: `limnifs-format`, `-core`, `-write`, `-crypto`, `-delta`, `-ec`, `limnifs-locator-*`, `limni` CLI, `limnifs-fuse`; `TODO.impl/` + `docs/` migrate here | one workspace = atomic PRs across crates, single lockfile |
| `limnifs/spec` | `SPEC.md`, `schema/*.fbs`, registries, conformance vectors + harness | spec-first: independently tagged (spec v0.1, v0.2…); third-party implementations subscribe without Rust code; consumers pin a spec tag |
| `limnifs/limnifs-py` | Python reference reader | independence is its purpose: written from spec only, no shared repo to peek at |
| `limnifs/limnifs-frozen2` | legacy Frozen2 read adapter | license-scan boundary: the only repo where third-party (MIT/Apache) DwarFS code may appear |
| `limnifs/limnifs.org` | the website (Astro 7 + Vite 8 + Tailwind 4 + Vue islands) | public face; renders spec from pinned tag, never copies it |
| `limnifs/.github` | org-level reusable workflows + org profile | one place for CI machinery shared across repos (owned by 13) |

Cross-repo SSOT rule: the schema lives in `limnifs/spec`; `limnifs/limnifs`
pins a spec tag and regenerates bindings in CI (diff gate). Nothing is
copied by hand between repos.

**Domains (registered):** `limnifs.org` (homepage, repo `limnifs/limnifs.org`,
component 14), with `limnifs.com` and `limnifs.net` held and redirecting to
`.org`.

| # | Component | Crate(s) | Owns | Does NOT own | Phase |
|---|---|---|---|---|---|
| 00 | [architecture](00-architecture/README.md) | docs only | Normative architecture: module interaction points, interface contracts, algorithm specs, comparison with extant filesystems | implementation, task state | 0 |
| 01 | [spec](01-spec/README.md) | `limnifs-format` + `spec/` | FlatBuffers schema, registries, feature flags, versioning, codegen | reader/writer behavior | 0 |
| 02 | [conformance](02-conformance/README.md) | `conformance/` | test vectors, harness, fuzz corpus, differential testing | implementation code | 0+ |
| 03 | [core-reader](03-core-reader/README.md) | `limnifs-core` | manifest parse, drop read, overlay resolution, tier-agnostic read path | writing, networking, FUSE | 0 |
| 04 | [writer-pipeline](04-writer-pipeline/README.md) | `limnifs-write` | chunking, classification, ingest, deepening, slab packing, GC | delta semantics, crypto primitives | 1 |
| 05 | [crypto](05-crypto/README.md) | `limnifs-crypto` | AEAD registry, key wrap, signatures, DMS primitives | where keys are stored (manifest = 01) | 2 |
| 06 | [deltas-overlays](06-deltas-overlays/README.md) | `limnifs-delta` | delta build, metadata flatten, turnover, chain GC | overlay *resolution* (read side = 03) | 2 |
| 07 | [erasure-coding](07-erasure-coding/README.md) | `limnifs-ec` | per-slab Reed-Solomon encode/decode/repair | placement policy (04), manifest layout (01) | 3 |
| 08 | [locators-streaming](08-locators-streaming/README.md) | `limnifs-locator-*` | locator trait + file/http/s3/ipfs plugins, CAR interop, mirror racing | what bytes mean (03) | 2–3 |
| 09 | [legacy-frozen2](09-legacy-frozen2/README.md) | `limnifs-frozen2` | read-only DwarFS Frozen2 adapter, one-way import | Frozen2 writing (never built) | 1 |
| 10 | [cli](10-cli/README.md) | `limni` | the `limni` binary UX, subcommands, machine-readable output | library logic (delegates to crates) | 1 |
| 11 | [mount](11-mount/README.md) | `limnifs-fuse` | FUSE daemon, composefs-style kernel path | caching policy (08), image semantics (03) | 1, 3 |
| 12 | [tebako-integration](12-tebako-integration/README.md) | tebako-side glue | press/mount consuming `.limni`, parity tests | LimniFS internals | 1 |
| 13 | [ci-releases](13-ci-releases/README.md) | `.github/workflows/` | GitHub Actions matrix, merge gates, fuzz/bench schedules, reproducible releases, SBOM, license scan | test content (02), code (all others) | 0+ |
| 14 | [website](14-website/README.md) | `limnifs/limnifs.org` (Astro 7) | limnifs.org: pages, design system, Vue islands, site CI/deploy | spec content (01), release artifacts (13), product code | 1 |

## 4. Task file format and lifecycle

Every task file is `{component-nn}/{nn}-{task-name}.md` with this header:

```
- **Status:** pending | in_progress | done | blocked
- **Phase:** 0–3
- **Depends on:** list of sibling task files
- **Design refs:** design doc sections
- **Acceptance:** verifiable criteria (usually conformance vectors)
```

Rules:

- Exactly one `in_progress` task per component at a time.
- A task is `done` only when its acceptance criteria are demonstrated (conformance vector passes, benchmark number recorded, test green) — never by claim.
- Task files are updated in place; completed tasks are kept, not deleted (SSOT for history).
- When a task reveals the component map is wrong, fix the map in this README first, then move tasks.

## 5. Definition of done (any component)

1. Conformance vectors covering the change pass in **both** the Rust crate and the Python reference reader.
2. `clippy::pedantic` clean; no new `unsafe`; public API documented.
3. Benchmarks show no regression beyond the component README's stated budget.
4. License scan: no GPL-3 introduced.
5. Spec updated first if the wire format changed (SSOT: code follows spec, never the reverse).
6. **GitHub Actions green on the full matrix** (linux + macOS, stable Rust; plus the component's specific legs: fuzz smoke, bench assertion, license scan) — and the task file links the run. A task with no CI evidence is `pending`, regardless of local results.
7. **No shims/stubs merged**: grep gate rejects `todo!`, `unimplemented!`, `FIXME`-without-issue, skipped tests without a linked task file.
