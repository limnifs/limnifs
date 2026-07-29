# 01 — spec (SSOT)

The single source of truth for the LimniFS wire format. Everything that two
implementations must agree on lives here and nowhere else.

- **Phase:** 0
- **Repo:** `limnifs/spec` (independently tagged; consumers pin a spec tag)
- **Crate/artifacts:** `limnifs-format` (generated bindings, regenerated in the consumer repo's CI), `spec/SPEC.md`, `schema/*.fbs`
- **Design refs:** §4 (identity), §5 (three layers), §9 (AEAD registry), §10.1 (locator registry)

## Responsibilities (MECE)

**Owns:**

- FlatBuffers schemas for manifest, filesystem metadata, slab index, delta manifests.
- All registries as *data*: AEAD IDs (0x01 XChaCha20-Poly1305, 0x02 AES-128-OCB, 0x03 AES-256-GCM, 0x04 Ascon-128a), codec IDs, locator scheme IDs, classifier class IDs, feature flags.
- Versioning policy: per-layer version numbers, feature-flag negotiation, unknown-flag behavior (clean error, never misread).
- Codegen: Rust bindings, Python reference bindings, Markdown registry tables, JSON schema of registries — all generated, never hand-maintained.

**Does NOT own:** reader behavior (03), writer behavior (04), crypto implementations (05), test execution (02).

## Contract

- The schema is authoritative prose-plus-schema: every field has a semantic definition in `SPEC.md`; the `.fbs` file and the prose are generated from one model file where they would drift.
- Semantic types are declared here (`DropId`, `SlabId`, `ManifestRoot`, `Tier` ∈ {epilimnion, metalimnion, hypolimnion}) and codegen emits newtypes, not aliases.
- Any registry addition is OCP-pure: append a row, regenerate, done. No consumer code changes.

## Invariants

- No field is ever repurposed; deprecation = new field + tombstone flag.
- Identity rule is enforced by the schema: representations (codec/crypto/EC) live in a `Representation` table separate from `DropId` references.

## Tasks

- [01-format-spec-v01.md](01-format-spec-v01.md)
- [01-flatbuffers-schema.md](01-flatbuffers-schema.md)
- [01-feature-flag-registry.md](01-feature-flag-registry.md)
