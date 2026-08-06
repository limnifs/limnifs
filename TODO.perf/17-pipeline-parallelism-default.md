# 17 — Pipeline parallelism default for cold-cache

- **Priority:** P2
- **Side:** LimniFS
- **Est. effort:** 1d

## Problem

`limnifs-write` ships a `pipeline-parallelism` feature (producer/consumer
channels) but defaults to the simple `par_iter().map(process_file)`
shape. The pipeline variant overlaps file reads with compression
across N read threads + M compress threads, which should win on
cold-cache or network-attached workloads.

## Fix

1. Benchmark both modes on cold cache (drop kernel page cache between
   runs) and warm cache. Find the crossover.
2. If pipeline wins on cold cache and ties on warm cache, make it the
   default.
3. If pipeline wins only on cold cache, leave the feature opt-in and
   document the crossover.

## Expected impact

- 1.5–2× on cold-cache workloads
- Tied on warm cache

## Acceptance

- [ ] Benchmarks on cold + warm cache
- [ ] Documented crossover point
- [ ] Default flipped if warranted
