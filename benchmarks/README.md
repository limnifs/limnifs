# LimniFS Benchmark Suite

Performance testing for LimniFS, modelled on the
[DwarFS benchmark suite](https://github.com/tamatebako/dwarfs-t/tree/main/benchmarks).

Compares LimniFS against other read-only filesystem / archive tools
**when they are available on the system**. The LimniFS benchmark
always runs; the cross-format benchmarks (tar+zstd, mksquashfs,
mkdwarfs) run only if the relevant tool is on `PATH`.

## Quick start

```sh
# Tiny synthetic dataset, 1 iteration — for CI smoke tests.
python3 benchmarks/run_benchmarks.py --quick

# Python source tarball, 3 iterations — typical dev check.
python3 benchmarks/run_benchmarks.py --datasets python --iterations 3

# All datasets (tiny, python, linux), 5 iterations — full release check.
python3 benchmarks/run_benchmarks.py --all
```

Outputs land in `benchmarks/results/bench_<timestamp>.{json,md}`.

## Datasets

| Name | Approx size | Source |
|---|---|---|
| `tiny` | 10 MB | synthetic, no download (text + random binary + 1000 small files) |
| `python` | 55 MB | Python 3.12 source tarball |
| `linux` | 1.2 GB unpacked | Linux 6.6 kernel source tarball |

Real datasets are downloaded on first use and cached under
`benchmarks/datasets/`. Subsequent runs reuse the cache.

## Operations

| Operation | LimniFS command | Other tools |
|---|---|---|
| create | `limni limn` | `tar -c`, `mksquashfs`, `mkdwarfs` |
| verify | `limni verify` | `dwarfsck` (DwarFS only) |
| extract | `limni extract` | `tar -x`, `unsquashfs`, `dwarfsextract` |
| cat | `limni cat` (sequential read of every file) | n/a |

Each operation runs N iterations; the report shows the median and
stdev.

## Output

JSON (machine-readable, for trend tracking):

```json
{
  "metadata": {
    "date": "2026-07-31T11:04:43Z",
    "platform": "Darwin",
    "machine": "arm64",
    "iterations": 3,
    "limnifs_version": "limni 0.1.0"
  },
  "results": [
    {
      "format": "limnifs",
      "dataset": "python",
      "create": { "median_seconds": 1.23, "output_size_bytes": 12345678, ... },
      "extract": { ... }
    }
  ]
}
```

Markdown (human-readable):

```markdown
### Dataset: `python` (~55 MB)

| Format | Create (s) | Verify (s) | Extract (s) | Cat (s) | Size (MB) |
|---|---|---|---|---|---|
| limnifs | 1.23 | 0.04 | 0.45 | 12.3 | 11.8 |
| tar+zstd | 0.98 | — | 0.31 | — | 12.1 |
| squashfs | 1.45 | — | 0.52 | — | 11.5 |
| dwarfs | 4.21 | — | 0.61 | — | 9.8 |
```

## Memory tracking

On Linux, `/usr/bin/time -v` is used to sample peak RSS for each
operation. On macOS, this is skipped (`time -v` is GNU-specific).

## CI

[`.github/workflows/benchmark.yml`](../.github/workflows/benchmark.yml)
runs the benchmark suite on every release tag, attaching the JSON +
Markdown reports to the GitHub Release. Compare across releases to
spot regressions.

## Adding a comparison

If you want to compare LimniFS against a tool not yet covered:

1. Add a `benchmark_<format>(source, workspace, iterations)` function
   that returns a `BenchmarkResult`.
2. Add it to the runner loop in `main()`.
3. Make it gracefully skip when the tool isn't installed (return
   `None` or raise `RuntimeError`).

The cross-format benchmarks are intentionally best-effort: if your
system doesn't have `mkdwarfs`, that column just doesn't appear in
the report.
