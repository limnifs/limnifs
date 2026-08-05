# 14 — Tournament short-circuit

- **Priority:** P1
- **Side:** LimniFS
- **Est. effort:** 3h (implemented)
- **Status:** Done (v0.2.22)

## Problem

`TournamentConfig.codecs` was a hint, not a behaviour. `process_file`
ran exactly one codec per chunk (the class's preferred codec) and
never iterated the configured list. The config gave users a knob that
did nothing — and there was no fast path for "LZ4 already crushes
this chunk, skip Brotli".

Result: on text workloads where LZ4 achieves < 25% ratio in
microseconds, we still ran the configured Brotli pass for slight
ratio gains that rarely mattered to the user's actual goal.

## Fix

`process_file` now resolves `tournament.codecs` to numeric ids once
at the top level (`write_directory_with_config`) and passes a
`TournamentSpec` into each rayon worker. Per-chunk compression goes
through `compress_chunk_with_tournament`:

1. **Binary chunks with `skip_for_binary`**: use `binary_codec`
   directly (matches v0.1 behaviour).
2. **Chunks below `min_size_threshold`**: use the class's preferred
   codec directly. Tournament setup cost dominates at small sizes.
3. **Otherwise**: iterate `tournament.codecs` in declared order,
   tracking the best compression. Whenever a codec achieves ratio
   ≤ `short_circuit_permille`, accept and stop.

The short-circuit threshold is per-mille (0..=1000):
- `0` disables short-circuit — every configured codec runs.
- `250` (default) accepts the first codec that hits 25% of original
  size. Highly compressible chunks accept LZ4 and skip Brotli.
- `500` is loose — accept almost any compression (max-write).

Per-profile thresholds:

| Profile          | Threshold | Rationale |
|------------------|----------:|-----------|
| `max-ratio`      |    0      | Try every codec, pick smallest. |
| `max-speed`      |  500      | Speed-leaning; LZ4 almost always wins. |
| `balanced`       |  250      | Accept fast codec when it gets < 25%. |
| `competitive`    |  250      | Same. |
| `max-read`       |  200      | Tighter — favor ratio for read-heavy. |
| `max-write`      |  500      | Speed-leaning (`skip_chunking` dominates). |
| `max-write-rw`   |  500      | Same. |
| `max-read-rw`    |  200      | Same as max-read. |
| `balanced-rw`    |  250      | Same as balanced. |

## What did NOT change

- **Wire format**: drop codec ids still come from the existing
  codec registry. No new codec ids introduced.
- **`process_whole_file_drop`**: unchanged. When a categorizer
  claims a file (FLAC for WAV, FSST+Brotli for CSV, RICEPP for
  FITS), the categorizer's pipeline runs and the chunk-level
  tournament is bypassed. Categorizers exist precisely because
  they beat general-purpose codecs on specific file types — the
  tournament would only get in the way.
- **`skip_chunking` path** (max-write): unchanged. Whole-file LZ4
  is already the fast path; tournament overhead (even with short-
  circuit) is unnecessary.

## Benchmark impact

On the synthetic `--quick` benchmark, the tournament doesn't show
large swings because most synthetic datasets are categorizer-routed
(CSV, FITS, WAV) or random/zeros (no compressible text). The win
appears on real-world text workloads — source trees, JSON, logs —
where FastCDC produces many text chunks and LZ4 hits < 25% on most
of them.

To see the win, run against a real source tree:
```
cargo run --release -p limnifs-bench -- run --datasets php --profile balanced,max-write
```

## Acceptance

- [x] `tournament.codecs` is actually iterated for chunks ≥ `min_size`
      and not skipped via `skip_for_binary`.
- [x] `short_circuit_threshold = 0` runs every codec (no short-circuit).
- [x] Per-profile thresholds reflect each profile's intent (max-ratio
      disables short-circuit; max-write is loosest).
- [x] 5 unit tests cover: short-circuit on compressible, no short-
      circuit when disabled, binary skip, small-chunk fallback, store
      fallback when no codec compresses.
- [x] Full workspace test suite still passes (579/579).
