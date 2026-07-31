# 06 — Delta builder

- **Status:** done — limnifs-write/src/delta_builder.rs
- **Phase:** 2
- **Depends on:** 04-ingest-epilimnion
- **Design refs:** §7 (delta manifests), §16 (open question 4)

## Goal

`DeltaBuilder::diff(base, next)`: tree diff → tree ops + drop references →
delta manifest with `base_root`; new content goes through the 04 pipeline,
unchanged content is referenced.

## Notes

- Rename handling per spec decision (01); compiled at build time if spec says remove+add.
- Deltas are normal images: pinnable, signable, verifiable — no special-casing in readers beyond chain walking (03).

## Acceptance

- Diff vectors: add/remove/replace/rename each produce minimal op sets; unchanged trees produce empty deltas.
- Delta-of-delta chains build and resolve correctly (with 03-overlay-resolver).
