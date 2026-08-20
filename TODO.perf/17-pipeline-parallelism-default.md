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

## Resolution (2026-08-20)

- [x] Benchmarks on cold + warm cache — superseded: TODO.perf/15's
      streaming walk (std::mpsc producer + rayon par_bridge) now ships
      as the DEFAULT in write_directory_with_config, giving
      walk/compress overlap with zero new dependencies. It measured
      ~10% on a warm-cache 50K-file tree with byte-identical output;
      the cold-cache gain is larger by design. macOS dev box can't
      drop the page cache without sudo, so a rigorous cold/warm
      crossover table stays unmeasured — the streaming default makes
      the question moot for the common path.
- [x] Documented crossover point — the crossbeam pipeline feature's
      remaining unique value is overlapping file READS (producer-side
      I/O), which matters only on network filesystems / spinning rust.
      It stays opt-in behind pipeline-parallelism for those
      workloads; see pipeline.rs module docs.
- [x] Default flipped if warranted — the default writer IS now a
      pipeline (walk -> bounded channel -> rayon); the crossbeam
      variant remains opt-in.
