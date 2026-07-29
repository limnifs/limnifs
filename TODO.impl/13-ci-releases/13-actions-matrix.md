# 13 — GitHub Actions matrix

- **Status:** pending
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
  `todo!()`) fails the gate — both runs linked here.
