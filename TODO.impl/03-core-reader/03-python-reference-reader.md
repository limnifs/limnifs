# 03 — Python reference reader

- **Status:** done — limnifs/limnifs-py repo (spec-only oracle)
- **Phase:** 0
- **Depends on:** 01-format-spec-v01, 03-manifest-parser
- **Design refs:** §11, §15 (Phase 0 exit)

## Goal

A second, independent LimniFS reader in Python, written **from the spec only**
(not from the Rust code), wired into the conformance harness.

## Notes

- Lives in its own repo **`limnifs/limnifs-py`** — independence is the point:
  no shared workspace with the Rust implementation, pinned to a `limnifs/spec`
  tag like any third-party consumer.
- This is the spec-sufficiency test: if the Python reader can't be built from SPEC.md alone, the spec is incomplete — fix the spec first.
- Deliberately naive implementation; performance is irrelevant here.
- Doubles as the differential-fuzzing oracle (02-fuzz-corpus).

## Acceptance

- Passes the full conformance suite alongside the Rust reader.
- Phase 0 exit gate: both readers green = spec v0.1 is implementable.
