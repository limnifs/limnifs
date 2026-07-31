# 13 — Merge gates and scheduled legs

- **Status:** done — phase-0-exit.yml + phase-1-exit.yml + phase-2-exit.yml all green
- **Phase:** 0, grows per phase
- **Depends on:** 13-actions-matrix, 02-conformance-harness
- **Design refs:** §15 (phase exits)

## Goal

Required-check configuration per phase; nightly fuzz windows (24h cargo-fuzz
across targets, malicious corpus replay); nightly benchmark assertions against
each component README's stated budget; phase-exit aggregate jobs.

## Notes

- Phase-exit jobs are named and blocking: `phase-0-exit` (conformance on both
  readers + spec lint), `phase-1-exit` (tebako parity suite green), etc.
- Bench assertions fail on regression beyond budget, not just record — budgets
  live in component READMEs, this leg enforces them.
- Fuzz crashes auto-file issues with the minimized input attached; every
  crash becomes a permanent regression vector (02 rule) before the fix merges.

## Acceptance

- Each phase-exit job exists and blocks merges to main while red; mutant
  bench regression (artificial 2× slowdown on a branch) fails the nightly
  assertion — linked run as evidence.
