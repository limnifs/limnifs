# 01 — spec (SSOT)

The single source of truth for the LimniFS wire format. Everything
that two implementations must agree on lives here and nowhere else.

- **Phase:** 0
- **Repo:** `limnifs/spec` (independently tagged; consumers pin a spec tag)
- **Crate/artifacts:** `limnifs-format` (Rust custom serializer/deserializer
  derived from the spec via `serde`), multi-file spec under `spec/`
  (onion-layered — see [01-spec-restructure-plan.md]), registries as
  TOML data.
- **Design refs:** original design doc
  ([2026-07-28-limnifs-design.md](../../docs/superpowers/specs/2026-07-28-limnifs-design.md));
  wire format pivot
  ([2026-07-29-wire-format-pivot.md](../../docs/superpowers/specs/2026-07-29-wire-format-pivot.md)).

## Responsibilities (MECE)

**Owns:**

- **Custom wire format** for manifest, filesystem metadata, slab index,
  delta manifests. No FlatBuffers, no Avro, no Cap'n Proto (per pivot
  D1). Schema source = SPEC.md (multi-file, bit-level).
- **Deterministic Merkle B-tree** specification for the directory tree
  (per pivot D2). Prolly-inspired; spec-pinned split rules per §1.4
  determinism.
- All registries as *data*: AEAD IDs (0x01 XChaCha20-Poly1305, 0x02
  AES-128-OCB, 0x03 AES-256-GCM, 0x04 Ascon-128a), codec IDs, locator
  scheme IDs, classifier class IDs, feature flags.
- Versioning policy: per-section / per-blob version bytes (per pivot
  D3), feature-flag negotiation, unknown-flag behavior.
- Codegen: Rust types via `serde` (primary); Python reference bindings
  implemented from spec; Ruby/TS adapters via spec-only OR Rust
  FFI/WASM wrap (per pivot D5).

**Does NOT own:** reader behavior (03), writer behavior (04), crypto
implementations (05), test execution (02).

## Contract

- The spec is authoritative prose-plus-bit-level: every field has a
  semantic definition in the prose (Layer 1–2) and an exact byte/bit
  layout in the bit-level layer (Layer 3). The Rust crate
  `limnifs-format` implements both.
- Semantic types are declared in the spec (`DropId`, `SlabId`,
  `ManifestRoot`, `Tier ∈ {epilimnion, metalimnion, hypolimnion}`,
  `Representation`, `SlabRef`) and emitted as Rust newtypes via serde,
  not aliases.
- Any registry addition is OCP-pure: append a row to the TOML file,
  regenerate, done. No consumer code changes.
- File extension: `.lim` (per pivot D4).

## Invariants

- No field is ever repurposed; deprecation = new field + tombstone flag.
- Identity rule is enforced by the spec: representations
  (codec/crypto/EC) live in a `Representation` record separate from
  `DropId` references.
- Within a version byte, every field is fixed-width and order-pinned
  (per pivot D3). No vtable indirection; no per-record overhead.

## Tasks

- [01-format-spec-v01.md](01-format-spec-v01.md) — original single-file
  spec task (largely done; SPEC.md is 1359 lines covering Parts I–VII).
- [01-flatbuffers-schema.md](01-flatbuffers-schema.md) — **DEPRECATED**
  (FlatBuffers dropped per pivot D1; file kept per never-delete rule).
- [01-feature-flag-registry.md](01-feature-flag-registry.md) — produces
  `registries/*.toml` data files matching the §9 registry format.
- [01-spec-restructure-plan.md](01-spec-restructure-plan.md) — multi-
  file spec restructure (per pivot D6). The main upcoming work.
- _01-wire-format.md_ — TODO; replaces `01-flatbuffers-schema.md` as
  the custom wire format task. Opens once the restructure plan is
  approved.
