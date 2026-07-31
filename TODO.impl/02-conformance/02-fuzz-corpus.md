# 02 — Fuzz corpus and differential testing

- **Status:** done — 9 cargo-fuzz targets in fuzz/, nightly-fuzz.yml CI workflow, fuzz/README.md
- **Phase:** 0, ongoing
- **Depends on:** 02-conformance-harness
- **Design refs:** §11 (malicious-image corpus)

## Goal

cargo-fuzz targets for manifest parse, slab index walk, overlay resolution;
differential fuzzing Rust vs. Python reader on the same mutated inputs.

## Notes

- Seed corpus = conformance vectors; mutation via structure-aware fuzzer where possible (FlatBuffers-aware).
- Every crash → permanent regression vector before fix (root README §5).

## Acceptance

- 24h CI fuzz window green; zero panics on the malicious corpus (truncation, overlap, cycles, AD confusion).
