# 03 — Multi-profile benchmark

- **Priority:** P0
- **Side:** LimniFS
- **Est. effort:** 3h

## Problem

Current benchmark runs one profile (`balanced`). LimniFS has 9
profiles with different speed/ratio tradeoffs. The benchmark
should exercise at least 3: `max-write`, `balanced`, `max-ratio`.

## Fix

Add `--profile <name>` flag to `limnifs-bench run`. Run the full
benchmark matrix for each requested profile. Report side-by-side
in the markdown output.

## Expected impact

- Users can see the full speed/ratio tradeoff and pick the right
  profile for their workload.
- Shows that `max-write` is competitive with SquashFS on create
  speed (within 2× rather than 100×).

## Acceptance

- [x] `limnifs-bench run --profile max-write,balanced,max-ratio`
      produces a multi-profile report.
- [x] Each profile appears as a separate row in the report.

## Implementation notes (2026-08-05)

Shipped in v0.2.20.

- `runners::limnifs_create/verify/extract` take `profile_name: &str`
  and resolve the `WriteConfig` via `limnifs_write::profile::select`.
  Format tag becomes `limnifs:{profile}` so summaries are
  distinguishable in the report.
- Each profile writes to `limnifs-{profile}.lim` in the dataset work
  directory so multi-profile runs do not collide. External formats
  (DwarFS, SquashFS, tar+zstd) run once per dataset — they do not
  depend on LimniFS profile choice.
- `report::derive_formats` discovers the format list from results.
  LimniFS profiles sort first (plain `limnifs` before `limnifs:*`
  variants, then by name), then external formats in canonical order.
  Win/loss matrix and per-operation tables both use the dynamic list.
- Single-file ops (`extract_one`, `locate_one`, `read_random`) run
  against the primary profile's image (first in the `--profile` list,
  defaulting to `balanced`). These operations measure CLI overhead
  more than codec choice, so running them once is sufficient.
