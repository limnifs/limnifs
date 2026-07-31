# 09 — Frozen2 read adapter

- **Status:** deferred — user direction 2026-07-31: LimniFS is a separate filesystem; one-way migration is not v1 scope
- **Phase:** 1
- **Depends on:** 03-core-reader (traits)
- **Design refs:** §5.1, §1 (GPL boundary)

## Goal

Read DwarFS v2 (Thrift/Frozen2) images and expose them through `Image` /
`DropSource` so mounters treat them like native images.

## Notes

- Clean-room implementation; MIT/Apache `oxalica/dwarfs` reader may be vendored (license-compatible). GPL code is forbidden everywhere including here.
- Read-only by construction: no write API exists in this crate.
- Separate CI license scan for this crate (vendored code allowed: MIT/Apache only).

## Acceptance

- Mounts and reads tebako's real published DwarFS images byte-exactly (fixture set from tebako releases).
