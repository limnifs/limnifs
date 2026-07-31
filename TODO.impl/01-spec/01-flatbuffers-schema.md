# 01 — FlatBuffers schema + codegen

- **Status:** superseded — wire-format pivot 2026-07-29 (D1): custom format, not FlatBuffers
- **Phase:** 0
- **Depends on:** 01-format-spec-v01
- **Design refs:** §5, §14

## Goal

`schema/*.fbs` for manifest, fs metadata, slab index, delta manifest; plus the
codegen pipeline emitting Rust bindings (`limnifs-format`), Python reference
bindings, and registry Markdown/JSON.

## Notes

- Semantic newtypes (`DropId`, `SlabId`, `ManifestRoot`, `Tier`) generated, not hand-written (model-driven).
- `Representation` table separate from identity references (enforces design §4 structurally).
- Codegen is reproducible: pinned flatc version, checked-in generated code, CI diff gate.

## Acceptance

- `cargo build -p limnifs-format` and the Python bindings generate byte-identical output from the same schema commit.
- Registry docs render from schema annotations with zero hand edits (DRY/SSOT).
