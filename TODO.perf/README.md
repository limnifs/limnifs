# LimniFS Performance TODO — LimniFS-side items

These are performance improvements we implement in LimniFS code.
omnizip-side items are in separate files (prefixed `omnizip-`).

## Priority legend

- **P0** — measurable benchmark impact; ships next.
- **P1** — meaningful improvement; ships this cycle.
- **P2** — future; needs spec or measurement first.

## Index

| # | Title | Priority | Est. effort | Category | Status |
|---|---|---|---|---|---|
| 01 | [Metadata blob: zstd instead of Brotli](01-metadata-zstd.md) | P0 | 1h | Ratio + Speed | Done (v0.2.17) |
| 02 | [Wire CachedSlabStore into CLI](02-wire-cached-slabstore.md) | P0 | 2h | Read speed | Done (v0.2.18) |
| 03 | [Multi-profile benchmark](03-multi-profile-benchmark.md) | P0 | 3h | Benchmarking | Done (v0.2.20) |
| 04 | [Per-profile inline_threshold](04-inline-threshold-tuning.md) | P1 | 1h | Ratio (tiny-files) | Tested, reverted (no gain) |
| 05 | [posix_fadvise prefetch on slab files](05-slab-prefetch.md) | P1 | 2h | Read speed | Done (v0.2.19, madvise on macOS) |
| 06 | [FastCDC SIMD gear hash](06-fastcdc-simd.md) | P1 | 3d | Create speed | Done (v0.2.24, 4× unroll; full SIMD requires algo change) |
| 07 | [Parallel slab assembly](07-parallel-slab-assembly.md) | P2 | 2d | Create speed | Done (v0.2.24) |
| 08 | [Categorizer early-exit optimisation](08-categorizer-early-exit.md) | P1 | 1h | Create speed | Done (v0.2.18) |
| 14 | [Tournament short-circuit](14-tournament-short-circuit.md) | P1 | 3h | Create speed | Done (v0.2.22) |
| 15 | [Streaming directory walk](15-streaming-directory-walk.md) | P1 | 4h | Create speed | Pending |
| 16 | [Parallel manifest assembly](16-parallel-manifest-assembly.md) | P1 | 4h | Create speed | Pending |
| 17 | [Pipeline parallelism default](17-pipeline-parallelism-default.md) | P2 | 1d | Create speed | Pending |
| 18 | [mmap on the output side](18-mmap-output.md) | P2 | 6h | Memory | Pending |
| 19 | [BLAKE3 hash-tree parallelism](19-blake3-hash-tree.md) | P2 | 1d | Create speed | Pending |
| 20 | [Parallel slab decode (read side)](20-parallel-slab-decode.md) | P1 | 1d | Extract speed | Pending |
| 21 | [Drop-record batch encoding](21-drop-record-batching.md) | P2 | 3h | Create speed | Done (v0.2.38) |
| 22 | [Arc<Vec<u8>> for compressed bytes](22-arc-compressed-bytes.md) | P1 | 6h | Memory | Pending |
| 23 | [Write-side overlay layering](23-write-side-overlay-layering.md) | P0 | 1d | Architecture | **Done (v0.2.37)** |
| 24 | [`limni inspect`](24-cli-inspect.md) | P1 | 4h | UX | Done (pre-existing) |
| 25 | [Sign-then-verify CLI workflow](25-sign-verify-workflow.md) | P1 | 6h | Security | Pending |

## Omnizip-side items (filed as proposals)

| # | Title | Priority | Est. effort | Category |
|---|---|---|---|---|
| 09 | [omnizip: FLAC LPC encoder speed](09-omnizip-flac-speed.md) | P0 | 5d | Create speed |
| 10 | [omnizip: ricepp encoder speed](10-omnizip-ricepp-speed.md) | P0 | 3d | Create speed |
| 11 | [omnizip: ZSTD SIMD encode](11-omnizip-zstd-simd.md) | P1 | 5d | Create speed |
| 12 | [omnizip: Brotli SIMD encode](12-omnizip-brotli-simd.md) | P2 | 5d | Create speed |
| 13 | [omnizip: PPMd context tree init](13-omnizip-ppmd-init.md) | P2 | 3d | Create speed |
