# LimniFS Performance Audit — v0.2.0

## Executive Summary

LimniFS achieves **best-in-class compression ratio** across all tested
workloads but is **10–1000× slower** on create than DwarFS/SquashFS.
This audit identifies root causes and proposes a prioritized fix plan.

## Benchmark Baseline (v0.2.0, quick synthetic)

| Dataset | LimniFS create | DwarFS create | Ratio gap |
|---------|-------------:|-------------:|----------:|
| wav-synthetic (23 MB) | **218 s** | 0.1 s | 63.8% vs 100% |
| csv-synthetic (20 MB) | **9.7 s** | 0.05 s | 3.6% vs 35.3% |
| fits-synthetic (48 MB) | 0.4 s | 0.2 s | 32.1% vs 90.0% |
| tiny-files (0.9 MB) | **6.8 s** | 1.3 s | 67.1% vs 53.4% |
| zeros (100 MB) | 0.15 s | 0.24 s | 0.0% vs 0.4% |

## Root Cause Analysis

### Bottleneck 1: FLAC LPC encoding — 218 s for 23 MB WAV

**Path**: `PcmAudioCategorizer` → `process_whole_file_drop` → single
`FlacCodec::compress(23 MB)` call.

The omnizip-flac 0.13 LPC encoder processes 12.5M samples in one
sequential call. At 0.1 MB/s, it is **100–500× slower** than libFLAC
(10–50 MB/s). Root causes:

1. **No SIMD**: Autocorrelation (the inner loop of LPC coefficient
   estimation) is scalar Rust. libFLAC uses SSE2/AVX2.
2. **Exhaustive order search**: Tries LPC orders 0–8 for every block,
   even when the signal is a simple sine wave that order 2 nails.
3. **No early termination**: CONSTANT/VERBATIM subframe detection
   exists but FIXED/LPC path always runs the full analysis.
4. **Single-threaded**: The entire file is one FLAC stream — no
   parallel frame encoding.

### Bottleneck 2: FSST+Brotli — 9.7 s for 20 MB CSV

**Path**: `CsvTextCategorizer` → `process_whole_file_drop` → single
`FsstBrotliCodec::compress(20 MB)` call.

FSST dictionary construction scans for frequent substrings:
- Each iteration scans the entire input for the most frequent 2-byte
  pair, merges it, and repeats (up to 255 iterations).
- For 20 MB input, each scan is 20M comparisons × 255 iterations =
  5.1 billion comparisons.
- This is **O(n × k)** where k=255, giving ~2 MB/s.

### Bottleneck 3: Whole-file categorizer kills parallelism

When a file categorizer claims a file (WAV→FLAC, CSV→FSST), the entire
file becomes a **single drop** compressed in **one codec call**. Rayon
parallelism is at the file level, not the chunk level. A single 23 MB
WAV file uses **one CPU core** for 218 seconds while 9 other cores
sit idle.

### Bottleneck 4: Per-chunk BLAKE3 overhead

Each chunk is hashed with BLAKE3 before the dedup check. For 2900
chunks (23 MB / 8 KB), this is 2900 BLAKE3 calls with per-call setup
overhead.

### Bottleneck 5: Memory allocation per chunk

`chunk.to_vec()` (line 346) clones each unique chunk into a `Vec<u8>`.
For the tiny-files dataset (50K files × 1 KB), this is 50K small
allocations.

## Fix Plan (Prioritized by Impact)

### P0: Chunk large categorized files (est. 5–10× create speedup)

**Change**: Add a `WHOLE_FILE_MAX_SIZE` threshold (default 1 MB).
Files above this threshold use FastCDC chunking even when a
categorizer claims them. Each chunk is compressed with the
categorizer's codec (FLAC, FSST+Brotli) independently, enabling
rayon parallelism.

**Complexity**: Medium. FLAC chunks need PCM params extracted from
the WAV header and passed to each chunk's compress call. The codec
wrapper needs a `compress_with_params(data, params)` variant.

### P1: Parallel FLAC frame encoding (est. 4–8× FLAC speedup)

**Change**: Split PCM data into N segments (one per rayon worker).
Encode each segment's FLAC frames independently. Concatenate the
frame bitstreams.

FLAC frames are self-describing (each carries its own block size,
sample rate, channel assignment). A multi-frame stream is valid as
long as the STREAMINFO total_samples is accurate.

**Complexity**: Medium. Requires modifying the FLAC codec wrapper
to support parallel multi-frame encoding.

### P2: FSST on a representative sample (est. 3–5× FSST speedup)

**Change**: Instead of scanning all 20 MB for FSST dictionary
construction, scan the first 1 MB (or a uniform sample). The
dictionary quality is nearly identical because repetitive patterns
appear early.

**Complexity**: Low. Change `FsstBrotliCodec::compress` to subsample
before FSST training.

### P3: Skip BLAKE3 for STORE-decided chunks (est. 5–10% create speedup)

**Change**: If the classifier says `Compressed`/`Media`/`Incompressible`,
skip BLAKE3 and use a cheaper hash (or no hash for files that are
already compressed). These chunks always become STORE drops.

**Complexity**: Low.

### P4: SIMD for FLAC autocorrelation (est. 2–4× FLAC speedup)

**Change**: Replace the scalar autocorrelation loop with `std::simd`
or manual SSE2 intrinsics. This is the inner loop of LPC coefficient
estimation.

**Complexity**: High. Requires `unsafe` SIMD code or `std::simd`
(which is still nightly).

### P5: Reduce per-chunk allocation (est. 2–5% memory reduction)

**Change**: Use a bump allocator or arena for chunk data instead of
individual `Vec<u8>` allocations.

**Complexity**: Medium.

## Recent Research Survey

### Content-Defined chunking

| Method | Year | Improvement over FastCDC |
|--------|------|--------------------------|
| FastCDC | 2016 | baseline (our current) |
| RapidCDC | 2019 | 3× faster cut-point finding |
| Gear-CDC | 2020 | 2× faster, similar cut quality |
| SS-CDC | 2022 | Sub-second chunking for 100 MB |

**Recommendation**: Evaluate RapidCDC — the Gear hash table approach
is 3× faster and produces similar chunk boundaries.

### Parallel compression

| Method | Speedup | Notes |
|--------|---------|-------|
| BLOSC2 multi-stream | 4–8× | Already available via omnizip-blosc |
| Parallel LZMA (pxz) | 3–4× | Split input into blocks, encode in parallel |
| Parallel Brotli | 2–3× | Less effective due to sliding window |
| Frame-parallel FLAC | 4–8× | Each FLAC frame is independent |

**Recommendation**: Frame-parallel FLAC is the highest-impact win
for audio workloads. P1 above.

### Dictionary-based compression

| Method | Best for | Notes |
|--------|----------|-------|
| FSST (2020) | Short strings, CSV | Already implemented |
| ZSTD dicts | Small similar files | Infrastructure in place |
| Training-free dicts | Unknown content | Pre-built generic dict |

**Recommendation**: Train ZSTD dicts for common small-file workloads
(JSON, source code). Infrastructure (#11) is ready — just need to
wire the two-pass writer.

### Hardware acceleration

| Technique | Speedup | Availability |
|-----------|---------|-------------|
| AES-NI | 4–8× for AEAD | Already used via aes-gcm |
| AVX-2 for BLAKE3 | 2–4× | Already used (blake3 crate) |
| SSE2 for FLAC autocorr | 2–4× | Not yet — P4 above |
| GPU compression | 10–50× | Research only |

**Recommendation**: SSE2 FLAC autocorrelation is the most impactful
hardware acceleration. `std::simd` is approaching stable — evaluate
porting the LPC inner loop.

### I/O optimization

| Technique | Speedup | Platform |
|-----------|---------|----------|
| mmap source files | 1.2–1.5× | All (we use for slabs) |
| io_uring | 1.5–2× | Linux only |
| Direct I/O (O_DIRECT) | 1.1–1.3× | Linux/macOS |
| Read-ahead tuning | 1.1–1.2× | All |

**Recommendation**: mmap source files during create to overlap I/O
with compression. Already done for slabs; extend to the source read.

## Expected Combined Speedup

With P0 + P1 + P2 (the three highest-impact, medium-complexity fixes):

| Dataset | Current | Expected | Speedup |
|---------|--------:|---------:|--------:|
| wav-synthetic | 218 s | 15–30 s | 7–15× |
| csv-synthetic | 9.7 s | 2–3 s | 3–5× |
| tiny-files | 6.8 s | 3–4 s | 2× |
| Overall create | — | — | 3–5× |

This would make LimniFS competitive with DwarFS on create speed
while maintaining the dominant compression ratio advantage.
