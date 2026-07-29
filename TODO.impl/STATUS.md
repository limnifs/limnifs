# LimniFS — STATUS

Living log of work sessions. Newest entry on top. Each entry: what's done
(with CI links), what's in_progress, blockers, next.

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
