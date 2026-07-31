# 13 — GitHub Actions matrix

- **Status:** done — limnifs/.github repo reusable workflows + per-repo callers
- **Phase:** 0
- **Depends on:** none (stands up with the repo skeleton, before 01 lands)
- **Design refs:** §11; root README §5 items 6–7

## Goal

Core CI for the org: reusable workflows in `limnifs/.github` consumed by all
repos — cargo build/test/clippy/fmt/doc on {ubuntu-latest, macos-latest} ×
stable Rust (`limnifs/limnifs`, `limnifs/limnifs-frozen2`); Python leg
(`limnifs/limnifs-py`); spec lint + vector generation (`limnifs/spec`);
conformance harness leg; no-shims grep gate; codegen diff gate (spec tag
pinned, bindings regenerated and diffed).

## Notes

- Pinned action SHAs, `permissions: contents: read` default, locked toolchain
  via `rust-toolchain.toml`; per-repo workflows are thin callers of the
  org-level reusable ones (DRY across repos).
- The no-shims gate: ripgrep for `todo!|unimplemented!|#\[ignore\]` (without
  linked task comment) fails the build — anti-stub rule enforced mechanically.
- Cache discipline: keyed on lockfile + toolchain; no cache poisoning between
  branches.

## Acceptance

- Empty-skeleton repo goes green end-to-end; a mutant commit (injected
  `todo!()`) fails the gate — both runs linked below.

## Evidence

- **Green (empty skeleton)** —
  https://github.com/limnifs/.github/actions/runs/30426996117
  PR [limnifs/.github#1](https://github.com/limnifs/.github/pull/1) (merged).
  The `.github` repo's self-caller runs the no-shims gate against its own
  contents (YAML only, no Rust); the gate passes in 4s.
- **Red (mutant `todo!()`)** —
  https://github.com/limnifs/limnifs/actions/runs/30432668400
  PR [limnifs/limnifs#2](https://github.com/limnifs/limnifs/pull/2) (closed
  unmerged). Adds `mutant-shim-probe.rs` containing `todo!()` with no
  same-line `TODO.impl/...` task reference. The no-shims gate fails in 3s
  with `::error::Shim/stub patterns found without a linked task file`.
  The Rust matrix legs still pass (no `Cargo.toml` → graceful skip),
  proving the gate is the failing check.
- **Per-repo callers wired** — PRs
  [limnifs/limnifs#1](https://github.com/limnifs/limnifs/pull/1),
  [limnifs/spec#1](https://github.com/limnifs/spec/pull/1),
  [limnifs/limnifs-py#1](https://github.com/limnifs/limnifs-py/pull/1),
  [limnifs/limnifs-frozen2#1](https://github.com/limnifs/limnifs-frozen2/pull/1),
  all merged; each repo's `main` now runs the matrix on push and PR.

## Follow-ups (tracked in 13-ci-releases)

- Pin action SHAs (currently `@v4` / `@v5` major-version tags) — extend this
  task's acceptance or open a sibling task once a SHA-pinning helper is chosen.
- Add `codegen-diff.yml` once `01-spec` lands FlatBuffers schema + bindings.
- Add `license-scan.yml` once product repos have dependencies.
- Add `conformance-harness.yml` once `02-conformance` lands vectors.
- Add caching (keyed on lockfile + toolchain) once `Cargo.lock` exists.
