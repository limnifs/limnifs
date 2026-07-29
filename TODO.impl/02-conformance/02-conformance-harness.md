# 02 — Conformance harness

- **Status:** pending
- **Phase:** 0
- **Depends on:** 02-test-vector-format
- **Design refs:** §11, §15

## Goal

The runner that executes vectors against an implementation (Rust crate,
Python reference, later third parties) and reports identity-level pass/fail.

## Notes

- Black-box: harness speaks the format, never links implementation code.
- Emits JUnit + JSON reports; wired into CI as the merge gate.
- Performance vectors included: read amplification and memory ceilings (03 budget) asserted, not just measured.

## Acceptance

- Both Rust and Python readers run the full suite in CI.
- A deliberately corrupted reader (mutant) fails the suite — harness has teeth.
