# 09 — import-dwarfs (one-way migration)

- **Status:** deferred — depends on 09-frozen2-reader; same user direction
- **Phase:** 1
- **Depends on:** 09-frozen2-reader, 04-ingest-epilimnion
- **Design refs:** §5.1, §15 (Phase 1 exit)

## Goal

`limni import-dwarfs`: re-encode a Frozen2 image into `.limni` via the 04
pipeline, preserving tree semantics (inodes, xattrs, timestamps) and recording
provenance in the new manifest.

## Notes

- One-way only: Frozen2 writing is never built (design §2 non-goals).
- Provenance field records source image hash → migration auditability.

## Acceptance

- Import of tebako fixture set produces `.limni` images whose trees are byte-identical to source mounts; provenance recorded; ratio ≥ source.
