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
| 06 | [FastCDC SIMD gear hash](06-fastcdc-simd.md) | P1 | 3d | Create speed | Pending |
| 07 | [Parallel slab assembly](07-parallel-slab-assembly.md) | P2 | 2d | Create speed | Pending |
| 08 | [Categorizer early-exit optimisation](08-categorizer-early-exit.md) | P1 | 1h | Create speed | Done (v0.2.18) |
| 14 | [Tournament short-circuit](14-tournament-short-circuit.md) | P1 | 3h | Create speed | Done (v0.2.22) |

## Omnizip-side items (filed as proposals)

| # | Title | Priority | Est. effort | Category |
|---|---|---|---|---|
| 09 | [omnizip: FLAC LPC encoder speed](09-omnizip-flac-speed.md) | P0 | 5d | Create speed |
| 10 | [omnizip: ricepp encoder speed](10-omnizip-ricepp-speed.md) | P0 | 3d | Create speed |
| 11 | [omnizip: ZSTD SIMD encode](11-omnizip-zstd-simd.md) | P1 | 5d | Create speed |
| 12 | [omnizip: Brotli SIMD encode](12-omnizip-brotli-simd.md) | P2 | 5d | Create speed |
| 13 | [omnizip: PPMd context tree init](13-omnizip-ppmd-init.md) | P2 | 3d | Create speed |
