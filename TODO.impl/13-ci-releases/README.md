# 13 — ci-releases

GitHub Actions infrastructure and release engineering across the `limnifs`
org. Everything must be fully working in CI — this component owns the
machinery that proves it. No shims, no stubs, no local-only claims: if it
isn't green here, it isn't done.

- **Phase:** 0+ (standing from the first commit)
- **Repos:** org-wide — reusable workflows in `limnifs/.github`, consumed by
  `limnifs/limnifs`, `limnifs/spec`, `limnifs/limnifs-py`, `limnifs/limnifs-frozen2`
- **Artifacts:** `.github/workflows/` (per repo) + reusable workflows (org repo), release tooling, SBOM/license configs
- **Design refs:** §11 (supply chain), §15 (phase exit gates); root README §5

## Responsibilities (MECE)

**Owns:**

- Org-level reusable workflows (`limnifs/.github`): build+test matrix
  (linux × macOS, stable Rust; Python leg in `limnifs-py`), conformance
  harness leg, clippy/fmt/doc gates, the no-shims grep gate (rejects
  `todo!`/`unimplemented!`/skipped tests without linked task), codegen diff
  gate (spec tag pinned in `limnifs/limnifs`, bindings regenerated and diffed).
- Cross-repo contract checks: `limnifs/limnifs` and `limnifs/limnifs-py` pin
  `limnifs/spec` tags; CI fails on drift; spec releases trigger downstream
  pin-bump PRs (automation, not humans).
- Scheduled legs: nightly fuzz windows (cargo-fuzz + malicious corpus),
  nightly benchmark assertions against component budgets.
- Release pipeline: reproducible builds of `limni` per platform, SBOM
  generation, license scan (no GPL-3 anywhere — hard fail; `limnifs-frozen2`
  scanned separately for its MIT/Apache-only vendoring rule), sigstore signing
  of release artifacts, crates.io publishing order.
- Merge gates: required checks per phase; phase-exit aggregate jobs (e.g.
  "phase-0-exit" = conformance green on Rust + Python + spec lint).

**Does NOT own:** test content and vectors (02), any product code, benchmark
definitions (component READMEs) — only their execution and enforcement.

## Contract

- Every task file's acceptance criteria must be expressible as a CI check;
  a criterion that cannot run in GitHub Actions is rewritten until it can.
- CI is self-testing: a deliberately broken change (mutant) fails the gate
  (proven once per workflow, recorded in the task file).
- Workflows are pinned (action SHAs), minimal-permission, and reproducible
  (locked toolchains); macOS and Linux legs are both required, never optional.

## Tasks

- [13-actions-matrix.md](13-actions-matrix.md)
- [13-merge-gates.md](13-merge-gates.md)
- [13-release-sbom.md](13-release-sbom.md)
