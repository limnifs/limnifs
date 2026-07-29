# 06 — Turnover (tier 3, full re-encode defrag)

- **Status:** pending
- **Phase:** 2
- **Depends on:** 06-metadata-flatten, 04-slab-packing-gc
- **Design refs:** §7 (tier 3), §3 (turnover vocabulary)

## Goal

`Turnover`: repack all referenced drops into fresh slabs, re-run deepening
policy, GC unreferenced drops, emit a standalone manifest with no external
references.

## Notes

- Composition, not duplication: drives 04's packer/GC and the deepening compactor; adds only orchestration.
- Cancel-safe: produces valid intermediate state; old image untouched until atomic manifest swap.

## Acceptance

- Post-turnover image is standalone (locator audit: no external slab refs) and resolves byte-identical tree.
- GC reclaim measured and recorded on the delta-chain vector set.
