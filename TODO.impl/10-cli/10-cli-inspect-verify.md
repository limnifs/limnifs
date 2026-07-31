# 10 — inspect / verify commands

- **Status:** done — verify, inspect, ls, cat, stat, tree, slab, gc, history, dedup, check, benchmark
- **Phase:** 1
- **Depends on:** 10-cli-skeleton
- **Design refs:** §9 (verify), §11

## Goal

Read-side commands: `ls`/`cat`/`stat`, `verify` (hash + AEAD + signature chain),
`manifest` (pretty-print + feature flags), `dms status`.

## Notes

- `verify` output states *what* was proven: drop hashes, AEAD tags, signature bundle, chain linkage — not just "OK".
- Works uniformly on native and Frozen2 images (via 09 adapter).

## Acceptance

- Tamper-injection fixtures produce precise failure reports naming the failing drop/signature.
