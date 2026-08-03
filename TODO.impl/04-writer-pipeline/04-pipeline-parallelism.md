# 04 — Pipeline parallelism (I/O overlap with compression)

- **Status:** pending (spec only this cycle)
- **Phase:** 2
- **Depends on:** 04-deepening-compactor, 03-async-slab-source
- **Design refs:** §6 (pipeline), 2026-throughput-roadmap.md §5
- **Priority:** P1

## Goal

The current writer is `par_iter().map(process_file)` — rayon fans
out across files, but each file does its own `std::fs::read` +
chunk + compress inside the worker. On slow disks (network
filesystems, spinning rust) the read blocks the worker; on fast
disks the worker is already saturating the CPU so the overlap
doesn't matter.

For RW workloads where files are written in small batches and the
caller wants low write latency, the current shape is fine. For
cold-cache create-large-image workloads, overlapping read I/O with
compression gives a real speedup.

## Design

```text
                    ┌─────────────┐
read I/O  ─────────►│  staging    │──► crossbeam channel ──┐
 (N threads)        │  (vec<u8>)  │                       │
                    └─────────────┘                       ▼
                                                ┌─────────────────┐
                                                │  compression    │
                                                │   (M threads)   │
                                                └─────────────────┘
                                                          │
                                                          ▼
                                                ┌─────────────────┐
                                                │  slab packing   │
                                                │   (1 thread)    │
                                                └─────────────────┘
```

- `Pipeline` struct owns the channels and a `WriteContext` for the
  final assembly.
- `Pipeline::run(sources: impl Iterator<Item = PathBuf>)` drives
  the three stages.
- Back-pressure: bounded channels (capacity ≈ 2 × thread count).

## Notes

- **Defer implementation until a benchmark shows the win.** The
  current `par_iter` shape is very fast on NVMe; pipeline
  parallelism's payoff is on cold reads and network-attached
  storage. The LimniFS campaign benchmarks on PHP/Python/Linux
  kernel source are warm-cache after the first run.
- The trait shape should match the `AsyncSlabSource` trait from
  `03-async-slab-source.md` so the same async primitive serves both
  read and write paths.

## Acceptance

- [ ] Spec exists (this doc).
- [ ] `Pipeline` skeleton exists with `todo!()` body and a
      feature-gated `pipe` module; does not affect default build.
- [ ] A benchmark mode in `limnifs-bench` toggles the pipeline on
      and off and reports the delta.
- [ ] Pipeline ships only after at least one dataset in
      `limnifs-bench` shows ≥ 15% wall-clock improvement.
