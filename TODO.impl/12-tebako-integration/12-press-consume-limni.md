# 12 — tebako press consumes .limni

- **Status:** dropped — user direction 2026-07-31: tebako integration is tebako's concern, not LimniFS's
- **Phase:** 1
- **Depends on:** 10-cli-limn-merge, 11-fuse-daemon
- **Design refs:** §15 (Phase 1 exit), §12

## Goal

`tebako press --format limni`: package apps with `.limni` images; runtime
mount via libtfs adapter (or direct static link of `limnifs-core`); existing
dwarfs-t path untouched during transition.

## Notes

- Adapter contract first: LimniFS plugs into libtfs like any format — no special-casing in tebako core.
- Transition reversible per component invariant: both paths shippable until parity is green for two releases.

## Acceptance

- A real tebako sample app presses and runs from a `.limni` image on Linux and macOS CI.
