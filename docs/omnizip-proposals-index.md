# omnizip-rs codec proposals — consolidated index

This is the master index of all codec proposals LimniFS has filed
for omnizip-rs. Each proposal is a standalone document with clean-
room basis, algorithm summary, phased implementation plan,
acceptance criteria, and estimated effort.

## How to read this

1. Pick a codec from the priority table below.
2. Read its proposal document (linked).
3. If it looks worth doing, the proposal has everything needed to
   start: algorithm sources, architecture sketch, LOC estimate,
   acceptance test definitions.

## Clean-room principle

**Algorithms are not copyrightable. Only specific code is.**
If omnizip reimplements an algorithm from a published specification
or academic paper — without reading the GPL/LGPL source — the
reimplementation is an independent work under MIT/Apache.

The constraint is: **implement from the spec, not from the source.**
A separate tester may run the GPL/LGPL binary as a black box for
differential comparison. No GPL code enters the Rust source tree.

## All proposals

### Tier 1 — omnizip already has these on the TODO list

| Proposal | Doc | Status |
|---|---|---|
| FLAC LPC encoder | `omnizip-0.5-followups.md` item P2 | omnizip TODO 62 |
| LZMA optimal parser | `omnizip-0.5-followups.md` item P1 | omnizip TODO 64 |
| ZSTD dictionaries | `omnizip-zstd-dictionaries-proposal.md` | Not yet filed by omnizip |

### Tier 2 — clean-room from public-domain specs or papers

| Codec | Proposal doc | Source basis | Est. effort | LimniFS win |
|---|---|---|---:|---|
| **ZPAQ** | `omnizip-zpaq-proposal.md` | Public-domain spec (`zpaq.pdf`) | ~3200 LOC, 4 weeks | Best archival ratio; beats LZMA by 10-20% |
| **PPMd** | `omnizip-ppmd-proposal.md` | DCC 2001 paper (Shkarin) | ~2200 LOC, 3.5 weeks | Text ratio; beats Brotli q11 by 5-15% |
| **GLZA** | `omnizip-glza-proposal.md` | Published format spec + paper | ~3200 LOC, 4 weeks | DNA/logs; grammar-based wins on hierarchical repetition |

### Tier 3 — new algorithm categories

| Codec | Proposal doc | Source basis | Est. effort | LimniFS win |
|---|---|---|---:|---|
| **BLOSC2** | `omnizip-blosc2-proposal.md` | BSD-3 spec (Blosc org) | ~1300 LOC, 2 weeks | Scientific floats: 80% → ~40% |
| **JPEG XL lossless** | (not yet written) | BSD-3 spec (JPEG) | ~20K LOC, 2-3 months | PNG archives -20% |

### Previously filed (addressed in omnizip 0.5-0.9)

| Proposal | Doc | Status |
|---|---|---|
| ZSTD Phase C encoder + FSE fix | `omnizip-0.4-followups.md` | ✅ Shipped in 0.5-0.7 |
| ZSTD Huffman literals | `omnizip-0.4-followups.md` | ✅ Shipped in 0.9 |
| LZMA match finder wiring | `omnizip-0.5-followups.md` | ✅ Shipped in 0.5 |
| FSST preprocessor | (verbal) | ✅ Shipped in 0.4 |
| Rice++ codec | (verbal) | ✅ Shipped in 0.4 |
| FLAC skeleton + PCM parsers | (verbal) | ✅ Shipped in 0.4 |
| FLAC FIXED encoder | (verbal) | ✅ Shipped in 0.9.1 |
| ZSTD level differentiation | `omnizip-0.5-followups.md` | ✅ Shipped in 0.7 |
| ZSTD package-merge Huffman | `omnizip-0.5-followups.md` | ✅ Shipped in 0.8.3 |

## Priority ranking (by ROI for LimniFS)

| Rank | Codec | Ratio win | Speed cost | Effort | When to do |
|---|---|---|---|---:|---|
| 1 | FLAC LPC encoder | Audio: 100% → ~17% | Moderate | Low (wiring in place) | Now — blocks audio use case |
| 2 | LZMA optimal parser | Text: matches/beats ZSTD | Slow (archival) | Low (already wired) | Now — unblocks LZMA routing |
| 3 | ZSTD dictionaries | tiny-files: 69% → ~25% | Fast | Medium | Next — biggest single win |
| 4 | **ZPAQ** | Archival: beats LZMA by 10-20% | Very slow | 4 weeks | After 1-3 |
| 5 | BLOSC2 | Scientific floats: 80% → ~40% | Fast | 2 weeks | When scientific users appear |
| 6 | **PPMd** | Text: beats Brotli by 5-15% | Slow | 3.5 weeks | When archival mode ships |
| 7 | JPEG XL lossless | PNG: -20% | Moderate | 2-3 months | When image use case materialises |
| 8 | **GLZA** | DNA/logs: -10% vs LZMA | Slow | 4 weeks | When genomics users appear |

## Codec ID allocation (LimniFS wire format)

| Id | Codec | Status |
|---|---|---|
| 0x00 | store | ✅ |
| 0x01 | LZ4 | ✅ |
| 0x02 | ZSTD | ✅ |
| 0x03 | XZ/LZMA | ✅ |
| 0x04 | Brotli | ✅ |
| 0x05 | DEFLATE | ✅ |
| 0x06 | Snappy | ✅ |
| 0x07 | FLAC | ✅ (FIXED-only; LPC pending) |
| 0x08 | Rice++ | ✅ |
| 0x09 | FSST+Brotli | ✅ |
| 0x0A | BLOSC2 | Reserved |
| 0x0B | ZPAQ | Reserved |
| 0x0C | PPMd | Reserved |
| 0x0D | GLZA | Reserved |

## What LimniFS does on its side

For each new codec omnizip ships:
1. LimniFS writes a ~30 LOC wrapper at `limnifs-core/src/codec/<name>.rs`.
2. LimniFS writes or updates a file-level categorizer at
   `limnifs-write/src/file_categorizer/<name>.rs` that detects the
   target file type and routes to the codec.
3. LimniFS registers the wrapper in the codec registry
   (`limnifs-core/src/codec/mod.rs`).
4. LimniFS adds a benchmark dataset if the codec targets a new
   content class.

No architectural changes needed — the framework (categorizer
registry, codec registry, wire format) is already in place from
session 34.

## References

- Boundary: `docs/omnizip-vs-limnifs-boundary.md`
- Algorithm survey: `docs/compression-algorithm-research.md`
- Benchmark expansion: `docs/benchmark-expansion-design.md`
- DwarFS investigation: `docs/dwarfs-multicodec-investigation.md`
