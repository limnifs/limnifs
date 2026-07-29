# 02 — conformance

The acceptance gate for every implementation. LimniFS is spec-first: an
implementation is correct iff it passes this suite. Nothing in any other
component is "done" unless the relevant vectors pass here.

- **Phase:** 0+ (grows every phase)
- **Repo:** `limnifs/spec` (vectors + harness ship with the spec; implementations run them via I12)
- **Artifacts:** `conformance/` — vector definitions (YAML/JSON), generator, harness, fuzz corpus
- **Design refs:** §11 (security model), §15 (phase exits)

## Responsibilities (MECE)

**Owns:**

- Test-vector *definitions* (model-driven: vectors are generated from declarative descriptions + the schema, not hand-built binaries).
- The harness that runs vectors against an implementation binary/library and diffs results.
- Differential testing: Rust crate vs. Python reference reader on identical inputs.
- Malicious-image corpus: truncated slabs, overlapping extents, overlay cycles, nonce/AD confusion attempts, unknown feature flags, hostile locator behavior.

**Does NOT own:** fixing implementations (the failing component does), the schema (01).

## Contract

- Vectors are content-addressed too: each vector names its expected `ManifestRoot`/`DropId`s, so a pass means *identity agreement*, not just "didn't crash".
- Every fuzz crash becomes a permanent regression vector before the fix lands.
- Performance vectors exist (read amplification ceilings, memory ceilings) — correctness includes resource bounds.

## Invariants

- The suite never imports implementation code; it speaks the format only (black-box by construction).
- A vector that cannot be regenerated from its declarative source does not exist.

## Tasks

- [02-test-vector-format.md](02-test-vector-format.md)
- [02-conformance-harness.md](02-conformance-harness.md)
- [02-fuzz-corpus.md](02-fuzz-corpus.md)
