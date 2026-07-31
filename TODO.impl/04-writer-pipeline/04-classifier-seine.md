# 04 — Seine classifier

- **Status:** done — limnifs-write/src/classifier.rs (6 entropy/magic-byte classes)
- **Phase:** 1
- **Depends on:** 04-chunking-fastcdc
- **Design refs:** §6 (classification), §1.2 (the core DwarFS idea)

## Goal

The classifier registry + first classifiers: entropy + magic-byte heuristics
labeling drop classes {text/code, binary, already-compressed, media, sparse}.

## Notes

- OCP: classifiers register behind `Classifier`; ordering and fallback rules in one place.
- Class labels recorded per drop class in metadata — never baked into layout (reclassification stays possible).
- This is the generalization of DwarFS's one-shot categorizer into a re-runnable pipeline stage.

## Acceptance

- Classification accuracy ≥ 95% on a labeled corpus (misclassification only costs ratio, never correctness — vector proves misclassified drops still round-trip).
