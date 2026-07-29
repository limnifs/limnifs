# 02 — Test-vector format and generator

- **Status:** pending
- **Phase:** 0
- **Depends on:** 01-flatbuffers-schema
- **Design refs:** §11, §15 (Phase 0 exit)

## Goal

Declarative vector description format (YAML) + generator that emits binary
`.limni` fixtures: minimal images, each codec, tier placement, delta chains,
encrypted, EC, unknown flags, truncated/corrupt variants.

## Notes

- Vectors are generated, never hand-built (model-driven); each names expected `ManifestRoot`/`DropId`s so passing means identity agreement.
- Golden outputs stored content-addressed in-repo (small) or via LFS (large).

## Acceptance

- Regeneration from definitions is byte-reproducible.
- Coverage matrix: every schema table and every registry row appears in ≥1 vector.
