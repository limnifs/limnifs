# 04 — Wire PPMd / LZMA / Bzip2 tunables through the writer

- **Status:** in_progress
- **Phase:** 1
- **Depends on:** 04-deepening-compactor, 04-specialized-codecs
- **Design refs:** §6 (pipeline), 2026-throughput-roadmap.md §3
- **Priority:** P0

## Goal

The profile system already declares per-codec tunables
(`CodecTunables { brotli, ppmd7, ppmd8, lzma, bzip2 }`) and the
parallel writer threads `brotli_quality` through. The other codec
families are silently dropped: `compress_with_options(codec, chunk,
quality)` only branches on Brotli and ZSTD; everything else falls
through to `compress(codec, plaintext)` which uses default
order/budget/dict.

Make `ppmd7.order + memory_budget_mb`, `ppmd8.order +
memory_budget_mb`, `lzma.dict_size_mb + lc/lp/pb +
use_optimal_parser`, and `bzip2.block_size_kb` actually reach the
codecs.

## Design (OCP, MECE)

1. Add a codec-agnostic `CodecTunables` struct to
   `limnifs-core::codec` carrying every known knob. Codecs read
   only the fields they understand; unknown fields are ignored.
2. Add a `Codec::compress_with_tunables(&self, plaintext,
   &CodecTunables)` trait method with a default impl that ignores
   tunables and calls `compress`. Override in PPMd7, PPMd8, LZMA,
   Bzip2.
3. Add a free function
   `limnifs_core::codec::compress_with_tunables(codec_id,
   plaintext, &CodecTunables)` dispatching via the registry.
4. In `limnifs-write`, add `WriteConfig::to_core_tunables(&self)
   -> limnifs_core::codec::CodecTunables` and thread it through
   `process_file` so the parallel writer calls
   `compress_with_tunables` instead of `compress_with_options`.
5. Keep `compress_with_options(codec, plaintext, quality)` as a
   thin compatibility shim that builds a `CodecTunables` with
   `quality` set and delegates.

The new code never touches Brotli or ZSTD paths; their behaviour is
unchanged because they don't override the default trait method when
the existing wiring suffices.

## Notes

- PPMd's `compress_with_budget(plaintext, order, budget)` is the
  right entry point in `omnizip-ppmd 0.13.1`. Verified.
- LZMA in `omnizip-lzma` exposes `compress_with_params`; check at
  implementation time.
- Bzip2 block size maps to the `900 KB..=9000 KB` range; validate
  via `Bzip2Tunables::block_size_kb`.

## Acceptance

- [ ] `limnifs_core::codec::CodecTunables` struct exists and is
      serialisable.
- [ ] PPMd7/PPMd8/LZMA/Bzip2 codec impls override
      `compress_with_tunables`.
- [ ] A new test in `limnifs-write` proves a `max-ratio` profile
      (which sets `ppmd7.memory_budget_mb = 256`) produces smaller
      PPMd output than the default 80 MB budget on a > 1 MB text
      fixture.
- [ ] All existing 535 workspace tests still pass.
- [ ] No public API removed; `compress_with_options` still works.
