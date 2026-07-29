# 10 — limn / deepen / merge / turnover commands

- **Status:** pending
- **Phase:** 1–2
- **Depends on:** 10-cli-skeleton, 04-ingest-epilimnion, 06-metadata-flatten
- **Design refs:** §6, §7

## Goal

The mutation commands: `limni limn` (build image), `deepen` (tier policy),
`merge --flatten`, `turnover` — each printing the resulting `ManifestRoot`.

## Notes

- Progress and I/O-budget flags surface 04's knobs; defaults sane for CI.
- Identity always visible: every mutation prints the new root hash (component invariant).

## Acceptance

- End-to-end: limn → read back → deepen → flatten a delta → turnover, all via CLI, verified by conformance vectors at each step.
