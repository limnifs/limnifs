---
Component: 04-writer-pipeline
Task: 04-fsst-preprocessor
Status: done (2026-08-02, session 35)
Depends on: 04-classifier-seine
Unblocks: —
Source: docs/dwarfs-multicodec-investigation.md (Tier 1)
Fix landed: limnifs-core/src/codec/fsst_brotli.rs — FsstBrotliCodec
  (id 0x09) composites omnizip-fsst + Brotli; falls back to plain
  Brotli when FSST doesn't help. Wired via csv_text categorizer.
  CSV benchmark: 3.57% (beats DwarFS 35%, SquashFS 16%).
---

# 04-fsst-preprocessor — FSST string-table preprocessor before Brotli

## Problem

DwarFS uses FSST (Fast Static Symbol Table) as a precompressor that
finds common substrings in a block and replaces each with a single
byte before the main compressor runs. Reported 1.2–1.5× ratio
improvement on text-heavy workloads (CSV, JSON, source code with
shared boilerplate).

LimniFS compresses each chunk directly with Brotli q5. We miss the
FSST preprocessing stage.

## Approach

Add a new composite codec representation in the drop record:
"fsst-brotli" — the bytes on the wire are FSST-compressed, then
Brotli-compressed. Reader reverses: Brotli decompress, then FSST
expand.

Wire format options:
- (A) New codec id 0x07 = FSST. The drop representation byte becomes
  `(fsst << 4) | brotli` (composite codec encoding).
- (B) Extend the representation triple with a "preprocessor" byte.
  Cleaner but a wire-format break.

Recommend **A** with a new `Codec::PreCompressor` trait — keeps the
existing registry shape, composes cleanly.

## Implementation sketch

1. Port FSST to Rust. Reference: the VLDB 2020 paper "FSST: the
   Fast Static Symbol Table". Existing C++ impl in DwarFS at
   `src/internal/fsst.cpp` (~500 LOC). Pure algorithm, no system
   deps. Lives in `omnizip-rs/omnizip-fsst` (propose to omnizip) or
   in `limnifs-core/src/codec/fsst.rs` (local).
2. Add `CodecId::FSST_BROTLI = 0x07` to the registry. Encode:
   FSST-compress → Brotli q5. Decode: Brotli decompress → FSST
   expand.
3. Classifier routing: add a heuristic for "repetitive text" (file
   extension or content sniffing for CSV/JSON/JS/TS) → route to
   FSST-Brotli instead of plain Brotli.

## Acceptance criteria

- New codec 0x07 round-trips (compress + decompress = identity).
- Benchmark on a CSV-heavy dataset (e.g. NYC Taxi data) shows
  ratio improvement ≥ 1.2× vs plain Brotli q5.
- No regression on PHP / Python source benchmarks (where FSST may
  not help much).
- Conformance vector added for the composite codec.

## CI evidence required

`benchmark.yml` full mode shows ratio improvement on a CSV dataset
in the report.
