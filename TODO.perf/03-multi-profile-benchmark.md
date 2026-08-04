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

- [ ] `limnifs-bench run --profile max-write,balanced,max-ratio`
      produces a multi-profile report.
- [ ] Each profile appears as a separate row in the report.
