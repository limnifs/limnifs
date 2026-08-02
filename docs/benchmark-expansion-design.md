# limnifs-bench v2: expanded measurement surface

After auditing the DwarFS benchmark suite at
`src/external/dwarfs-t/benchmarks/` (which the user wrote), LimniFS's
benchmark currently measures **5 of the 11 axes** DwarFS covers. This
doc designs the expansion.

## Current state

limnifs-bench today measures:
- ✅ create time
- ✅ extract time (full image)
- ✅ verify time
- ✅ output size (ratio)
- ✅ throughput (MB/s) — derived from create/extract time + input size

That's it. We're blind to:
- ❌ single-file extract latency
- ❌ locate-one (path resolution) latency
- ❌ random byte-range reads
- ❌ sequential byte-range reads
- ❌ stride byte-range reads
- ❌ CPU time (user + system)
- ❌ peak RSS memory
- ❌ cold vs warm cache

## DwarFS's measurement axes (the target)

From `benchmarks/libdwarfs/benchmark_framework.h`:

### Metrics collected per iteration

| Metric | Source | Notes |
|---|---|---|
| Wall time (median, mean, stdev, min, max) | `std::chrono::steady_clock` | The headline number. |
| CPU user time | `getrusage(RUSAGE_SELF).ru_utime` | Excludes system calls. |
| CPU system time | `getrusage(RUSAGE_SELF).ru_stime` | Kernel time. |
| RSS | `getrusage`.ru_maxrss (macOS) or `/proc/self/status` VmRSS (Linux) | Working set. |
| Virtual size | Same sources | Address space. |
| Peak RSS | Tracked across iterations | High-water mark. |
| Throughput | `data_size / median_time` | MB/s when applicable. |

### Operations

| Operation | What it measures | DwarFS bin |
|---|---|---|
| **Single file extract** | Latency: open image, find one file, extract bytes. | `single_file_bench` |
| **Full extract** | Throughput: extract every file. | `full_extract_bench` |
| **Multi-file extract** | Mixed: extract N files from a list. | `multiple_files_bench` |
| **Random access** | Read N random byte ranges from one file. | `random_access_bench` |
| **Sequential access** | Read sequentially from one file. | `random_access_bench` with `--pattern sequential` |
| **Stride access** | Read every Nth byte range. | `random_access_bench` with `--pattern stride` |

### Configuration axes

- Iterations (default 3) + warmup iteration (default 1)
- Cache size (default 512 MiB)
- Worker thread count (default 2)
- Image format (`.dff` vs `.dft` for DwarFS)

### Reported numbers (Perl 5.43.3 dataset, 96.5 MB, 6 816 files)

From `benchmarks/README.md`:

- **Single file** (48 KiB): cold 8.29 ms, warm 0.21 ms median, 16.05 MB/s, 144 KiB peak RSS, **39× speedup cold→warm**.
- **Full extract**: median 1.49 s (4 workers), mean 3.48 s, 27.75 MB/s, 8.44 MiB peak RSS, **5× speedup cold→warm**.

The cold-vs-warm split is the headline insight: first-time access is dominated by manifest parsing + slab opens; subsequent access is dominated by codec decode. The 5-39× speedup ratio is the metric readers actually care about.

## Design for limnifs-bench v2

### Expanded `OperationResult`

```rust
pub struct OperationResult {
    pub format: String,
    pub operation: Operation,
    pub success: bool,
    pub wall_secs: f64,
    pub cpu_user_secs: f64,
    pub cpu_system_secs: f64,
    pub peak_rss_bytes: u64,
    pub output_size_bytes: u64,
    pub items_processed: u64,
    pub bytes_processed: u64,
    pub cache_state: CacheState,
}

pub enum Operation {
    Create,
    Verify,
    ExtractFull,
    ExtractOne { path: String },
    LocateOne { path: String },                    // path resolve, no byte read
    ReadSequential { path: String, read_size: usize, num_reads: usize },
    ReadRandom { path: String, read_size: usize, num_reads: usize, seed: u64 },
    ReadStride { path: String, read_size: usize, stride: usize, num_reads: usize },
}

pub enum CacheState {
    Cold,  // first access after image open
    Warm,  // subsequent access (cache primed)
}
```

### Resource usage tracker (cross-platform)

Need a `ResourceSnapshot` type that captures CPU time + RSS at start
and end of each iteration:

```rust
pub struct ResourceSnapshot {
    pub user_secs: f64,
    pub system_secs: f64,
    pub rss_bytes: u64,
}

impl ResourceSnapshot {
    pub fn now() -> Self { /* getrusage on Unix; Job Info on Windows */ }
    pub fn delta_since(&self, earlier: &Self) -> ResourceDelta { /* */ }
}
```

Platform specifics:
- **macOS**: `getrusage(RUSAGE_SELF)` gives `ru_utime`, `ru_stime`, `ru_maxrss` (in bytes).
- **Linux**: `getrusage` gives time; `ru_maxrss` is in KiB on Linux (in bytes on macOS — inconsistent!). For peak RSS we should also parse `/proc/self/status` VmHWM for accuracy.
- **Windows**: `GetProcessTimes` for CPU; `GetProcessMemoryInfo` for `PeakWorkingSetSize`.

Single function, three platform `cfg!` branches. ~80 LOC.

### Operation implementations per format

For each operation × format pair, we need a runner:

| Operation | limnifs | dwarfs | squashfs | tar+zstd |
|---|---|---|---|---|
| Create | `write_directory` direct | `mkdwarfs` | `mksquashfs` | `tar` |
| Verify | `limni verify` | `dwarfsck` | — | — |
| ExtractFull | `limni extract` | `dwarfsextract` | `unsquashfs` | `tar -xf` |
| ExtractOne | `limni cat > /dev/null` | mount + `cp` | `unsquashfs -f file` | `tar -xf arch path` |
| LocateOne | `limni stat` | mount + `stat` | `unsquashfs -ll` | `tar -tvf arch path` |
| ReadSequential | `limni cat --offset --length` | mount + `dd if=... of=/dev/null bs=N` | mount + `dd` | extract then `dd` |
| ReadRandom | same + random offsets | mount + `dd skip=N` | mount + `dd` | extract then `dd` |

The "mount + ..." operations for dwarfs/squashfs require FUSE which
adds kernel overhead. DwarFS's benchmark also has this issue; they
work around it by also benchmarking the C++ API directly. LimniFS
could do the same via a Rust library API path (no subprocess).

For benchmark fairness, we should report **two numbers per format**:
- Subprocess (realistic for FUSE-based tools)
- Library API (best case for tools that expose one — LimniFS does
  via `limnifs-core`, DwarFS via `libdwarfs`)

### CLI flags to expose

```
limnifs-bench run --datasets php \
    --operations create,verify,extract_full,extract_one,locate_one,read_random \
    --iterations 5 \
    --warmup-iterations 1 \
    --read-size 4096 \
    --num-reads 1000 \
    --target-file /Zend/zend.c \
    --library-api
```

### Output schema (JSON)

Match DwarFS's JSON schema as closely as possible:

```json
{
  "date": "epoch:...",
  "platform": "macos aarch64",
  "datasets": [...],
  "results": [
    {
      "dataset": "php",
      "format": "limnifs",
      "operation": "extract_one",
      "cache_state": "cold",
      "target_path": "/Zend/zend.c",
      "iterations": 5,
      "time": {
        "median_secs": 0.008,
        "mean_secs": 0.009,
        "stdev_secs": 0.001,
        "min_secs": 0.007,
        "max_secs": 0.012
      },
      "cpu": {
        "user_secs": 0.005,
        "system_secs": 0.002
      },
      "memory": {
        "peak_rss_bytes": 150000
      },
      "items_processed": 1,
      "bytes_processed": 58250
    }
  ]
}
```

### Markdown report — per-operation tables + cold/warm split

The current report groups by dataset × operation. The v2 report
adds:

1. **Cold-vs-warm comparison** for each operation (the headline
   DwarFS metric).
2. **Memory + CPU columns** alongside time.
3. **Random access patterns** broken out by `read_size × num_reads`.

```markdown
### Single File Extract — cold cache

| Dataset | Format | Median (ms) | CPU (ms) | Peak RSS (KiB) | Throughput (MB/s) |
|---|---|---:|---:|---:|---:|
| php | limnifs | 8.29 | 5.1 | 144 | 7.0 |
| php | dwarfs  | 8.29 | 5.0 | 144 | 7.0 |

### Single File Extract — warm cache

| Dataset | Format | Median (ms) | CPU (ms) |
|---|---|---:|---:|
| php | limnifs | 0.21 | 0.18 |
| php | dwarfs  | 0.21 | 0.18 |

### Cold→Warm Speedup

| Dataset | limnifs | dwarfs | squashfs |
|---|---:|---:|---:|
| php | 39.5× | 39.5× | 12.3× |
```

### Implementation phases

1. **Resource usage tracker** (~80 LOC, 1 file). Cross-platform
   `getrusage` wrapper.
2. **Expand `OperationResult` + `BenchmarkSummary` model** to carry
   CPU/memory/cache-state. JSON schema update.
3. **Add `extract_one` + `locate_one` operations** for all 4 formats.
   Subprocess path only; library API later.
4. **Add `read_random`/`read_sequential`/`read_stride`** — requires
   `limni cat --offset --length` CLI extension. ~30 LOC change to
   limni.
5. **Cold/warm cache split** — run each operation twice; first is
   cold, second is warm.
6. **Markdown report v2** with the new tables.

Estimated effort: 2-3 days for the full surface. Phases 1-3 are
self-contained and shippable independently.

## Honest scope concerns

- **Library-API path**: LimniFS could win big here vs FUSE-mounted
  alternatives. But it requires building a `limnifs-bench-lib`
  binary that links limnifs-core directly. Different binary than
  the subprocess-based bench. Defer until subprocess path is
  validated.
- **Mount vs subprocess**: For dwarfs/squashfs, the "fair"
  comparison is FUSE mount + filesystem ops (what users actually
  do). Subprocess (`dwarfsextract`) is for batch extract, not
  random access. We need both.
- **Cache state semantics**: For LimniFS, "cold" = first call after
  process start (manifest parse + slab open). For dwarfs/squashfs,
  "cold" = first read after mount. Different cost models; the
  numbers aren't directly comparable across formats. Document this.

## References

- DwarFS benchmark source: `src/external/dwarfs-t/benchmarks/`
- DwarFS perf data (Perl 5.43.3): `benchmarks/README.md` lines 220-240
- DwarFS JSON schema: `benchmarks/schemas/{api,fuse}_benchmark.json`
- DwarFS perf framework: `benchmarks/libdwarfs/benchmark_framework.h`
- Current limnifs-bench: `limnifs-bench/src/{metrics,runners,report}.rs`
