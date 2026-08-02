# Compression algorithm research: what's new and what could LimniFS use

A 2026-timestamped survey of compression algorithms published or
significantly updated since 2020, evaluated for LimniFS integration.
For each: license, pure-Rust availability, use case, ratio/speed
tradeoff, and concrete next step.

## Selection criteria

LimniFS's hard constraints:
- **Pure Rust, no C deps** (air-gapped build).
- **No GPL-3 C/C++ code** linked into the build (license scan is a
  hard CI gate). **However**: clean-room reimplementations of
  algorithms described in GPL'd reference implementations are NOT
  bound by the original license. If omnizip reimplements an
  algorithm from the published specification (not from the GPL
  source), the reimplementation is independently licensed.
  Algorithms aren't copyrightable; only specific code is.
- **Deterministic output** (DropId = BLAKE3 of plaintext; images
  must reproduce byte-identically across runs and hosts).
- **Per-content-class fit** (we route by file type; codec must
  win on at least one class).

Soft constraints:
- Compression speed ≥ 50 MB/s on commodity hardware.
- Decompression speed ≥ 500 MB/s.
- Reasonable memory footprint (≤ 256 MiB working set).

## General-purpose lossless codecs

### Already integrated

| Codec | Source | Status |
|---|---|---|
| LZ4 | `lz4_flex` crate | Wired |
| ZSTD | `omnizip-zstd` 0.9.1 (real Phase C encoder) | Wired |
| Brotli | `brotli` crate | Wired (q5 default for source code) |
| DEFLATE | `miniz_oxide` | Wired |
| Snappy | `snap` via omnizip-snappy | Wired |
| XZ/LZMA | `omnizip-lzma` 0.9.1 (real Phase C encoder) | Wired |

### Worth investigating

#### **zstd 1.5.6+ dictionaries** (Facebook, BSD-3-Clause)

- **What**: Pre-trained dictionaries for small-file workloads
  (< 100 KiB files). Trained on a corpus; each dictionary ~110 KiB.
- **Status**: Underlying ZSTD is shipped via omnizip. Dictionary
  mode is a separate feature.
- **Pure Rust**: omnizip-zstd doesn't expose dictionary mode yet.
  Would need either upstream port or local implementation.
- **LimniFS win**: Huge on `tiny-files` dataset (currently 69%).
  Dictionaries could push to ~20% on identical-content tiny files
  by sharing a trained dictionary across all files of a class.
- **Action**: file omnizip issue. The dictionary loader is ~200
  LOC; training algorithm is another ~500 LOC.

#### **brotli 1.1+ custom dictionary** (Google, MIT)

- **What**: Brotli has a built-in static dictionary tuned for web
  content. Custom dictionaries can be appended at encode time.
- **Status**: `brotli` Rust crate may not expose the custom
  dictionary API.
- **LimniFS win**: Small. Brotli's built-in dict already nails
  web text. Custom dict would help non-web text (config files,
  logs) but the gain is marginal vs the implementation cost.
- **Action**: defer. Brotli q5 already wins on source code.

#### **LZ4 HC + LZ4 streaming** (LZ4_org, BSD-2)

- **What**: High-compression variant of LZ4. Same format, more
  thorough match finding. Ratio 5-15% better than LZ4 fast.
- **Status**: `lz4_flex` doesn't ship HC. The reference C library
  has it.
- **Pure Rust**: Would need porting (~1500 LOC for HC encoder).
- **LimniFS win**: Small. We use LZ4 for Binary class; HC would
  give 10% better ratio at 3-5× slower encode. Worth it only if
  Binary becomes a measured bottleneck.
- **Action**: defer.

### Specialized algorithm research (post-2020)

#### **PGLZ — PostgreSQL's LZ** (PostgreSQL, PostgreSQL License ≈ BSD)

- **What**: LZ variant tuned for short repetitive records. Used in
  TOAST for in-row compression of database columns.
- **Status**: Active in PostgreSQL 16. ~600 LOC of C.
- **Pure Rust**: Not available as a crate.
- **LimniFS win**: Minimal. Designed for ~2 KB records, not file
  archives.
- **Action**: skip. Wrong problem.

#### **FSE / tANS / rANS** (Facebook, MIT)

- **What**: Asymmetric Numeral Systems — entropy coder that beats
  Huffman on small alphabets with skewed distributions. Used by
  ZSTD (FSE), Facebook's Zstd, and the recent `rans-rs` crate.
- **Status**: omnizip-zstd already uses FSE internally. Could be
  used as a *standalone* codec for content where Huffman loses
  (small alphabets, e.g. XML tag streams).
- **Pure Rust**: `rans-rs`, `constriction` crates available.
- **LimniFS win**: Marginal standalone. FSE's win is inside a
  larger codec (ZSTD), not as an isolated layer.
- **Action**: skip standalone; FSE-as-ZSTD-component already
  captured.

#### **BLOSC / BLOSC2** (Blosc org, BSD-3)

- **What**: Multi-codec container specifically for **scientific
  data** (NumPy arrays, multi-dimensional grids). Splits data into
  chunks, applies shuffle/bitshuffle + a fast codec (LZ4, ZSTD).
- **Status**: BLOSC2 v2.x ships with `ndcodecs` (variable-bit
  compression for floats). 30K+ LOC C library.
- **Pure Rust**: Not available.
- **LimniFS win**: HUGE on FITS / scientific imaging. We use
  ricepp for integer-pixel data but BLOSC2 would crush
  floating-point data (which ricepp can't handle). DwarFS doesn't
  have BLOSC; LimniFS would be unique.
- **Action**: file omnizip issue. The shuffle (≈500 LOC) + LZ4
  inner codec (already have) gives most of the win. Full BLOSC2
  is large.

#### **ZFP** (LLNL, BSD-3)

- **What**: Lossy OR lossless compression for **multi-dimensional
  floating-point arrays**. Exploits the smoothness of physical
  simulation data. ~12K LOC C library.
- **Status**: Active. Used in scientific computing.
- **Pure Rust**: Not available.
- **LimniFS win**: Niche — climate models, CFD outputs. Probably
  too specialized.
- **Action**: skip unless scientific-data users materialize.

#### **SPDP / Short-Pattern Detecting Predictor** (Microsoft, MIT, 2023)

- **What**: Detects short repeating patterns (2-8 bytes) in
  structured data. Designed for trace files, log files, JSON.
- **Status**: Research paper; reference implementation available.
- **Pure Rust**: Not available.
- **LimniFS win**: Could help on CSV/JSON without FSST's overhead.
  But FSST already wins there.
- **Action**: skip. FSST covers this use case.

#### **DensiML / neural-network codecs** (various, 2023-2024)

- **What**: Use neural networks to model probability distributions
  for entropy coding. PPMd-with-ML.
- **Status**: Active research; no production codec shipped.
- **LimniFS win**: ZERO — neural codecs are non-deterministic
  across hardware. Violates content-addressing rule
  (`DropId = BLAKE3(plaintext)` requires byte-identical
  reproduction).
- **Action**: skip permanently. Documented as a violation.

#### **GLZA — Grammar-based LZ Compression** (Gregory G. Smith)

- **What**: Builds a context-free grammar describing the input.
  Excellent on highly-repetitive data (DNA, log files, source
  code with shared boilerplate).
- **Reference impl**: GPL-3 C code (~10K LOC).
- **Spec availability**: The algorithm is described in detail in
  Smith's academic paper and the `GLZA_format.md` doc in the repo.
  The format spec is sufficient for a clean-room reimplementation.
- **Clean-room status**: **VIABLE**. If omnizip implements from
  the spec/paper (not from the GPL source), the reimplementation
  is MIT/Apache-licensable. The algorithm itself is not patented.
- **LimniFS win**: Real for DNA/genomics and repetitive log
  workloads. Grammar-based compression is structurally different
  from LZ/dictionary approaches and wins on inputs with
  hierarchical repetition (e.g. XML with repeated tag structures).
- **Action**: **File omnizip issue.** Mark as "clean-room from
  spec, not from GPL source."

#### **ZPAQ — Context-Mixing Archiver** (Matt Mahoney)

- **What**: Context-mixing codec. Multiple models (order-0,
  order-1, word, match, LZP) contribute probability estimates
  that are mixed (via logistic mixing) and fed to an arithmetic
  coder. Best-in-class ratio on most data.
- **Reference impl**: GPL-3 C++ (~30K LOC).
- **Spec availability**: Mahoney published the full format
  specification as `zpaq.pdf` and `zpaq.txt`, placed in the
  **public domain**. The spec describes the bytecode VM, the
  model configuration format, and the container format.
- **Clean-room status**: **VIABLE**. The spec is public domain;
  a Rust implementation from the spec is independently licensed.
- **Determinism**: ZPAQ IS deterministic when the model
  configuration is pinned. If omnizip pins a specific config,
  the output is byte-reproducible.
- **LimniFS win**: **Best ratio in existence** for archival mode.
  ZPAQ typically beats LZMA by 10-20% on text and binary.
- **Action**: **File omnizip issue.** High-value for archival use
  case. ~3000 LOC for VM + standard models + container.

## Audio codecs

### Already integrated

- **FLAC** (omnizip-flac 0.9.1, FIXED-only encoder; LPC pending)
  - Status: real encoder shipped, LPC encoder TODO 62

### Worth investigating

#### **WavPack** (David Bryant, BSD-3)

- **What**: Lossless or lossy audio codec. Generally beats FLAC by
  2-5% on most material due to hybrid lossy/lossless mode and
  better joint-stereo handling.
- **Status**: Mature. libwavpack is ~25K LOC C.
- **Pure Rust**: Not available.
- **LimniFS win**: Marginal. FLAC is the industry standard; users
  expect FLAC files. WavPack would be a niche alternative.
- **Action**: skip unless audio becomes a major use case.

#### **TAK / TAK Lossless** (Thomas Becker, proprietary, 2008)

- **What**: Highest-ratio lossless audio. Comparable to WavPack.
- **Status**: Proprietary. Windows-only encoder.
- **Action**: skip permanently. License.

#### **MPC / Musepack** (Andree Buschmann, BSD-3, lossy only)

- **Action**: skip — lossy.

#### **Opus lossless mode** (IETF, BSD-3)

- **What**: Opus added lossless mode in 2023. Generally worse than
  FLAC but very fast decoder.
- **Status**: Active in libopus 1.4+.
- **Action**: skip. No ratio advantage.

## Image codecs

### Already integrated

- **Rice++** (omnizip-ricepp) — for FITS integer-pixel data.

### Worth investigating

#### **JPEG XL lossless mode** (JPEG, BSD-3)

- **What**: JPEG XL has a lossless mode that uses a reversible
  color transform + MA tree prediction + entropy coding. Typically
  10-25% better than PNG on natural images.
- **Status**: Reference implementation `libjxl` ships lossless
  mode. ~120K LOC C++.
- **Pure Rust**: `jxl-oxide` crate exists for decode (Apache-2.0);
  no encoder.
- **LimniFS win**: Big for PNG archives. We currently STORE PNG
  (already compressed). JXL lossless could recompress and beat
  by 20% on average.
- **Action**: file omnizip issue. The decoder exists in Rust;
  encoder is the hard part.

#### **WebP lossless** (Google, BSD-3)

- **What**: Lossless mode of WebP. Uses spatial prediction +
  LZ4-like encoding. Generally between PNG and JXL.
- **Status**: Mature. `libwebp` ~80K LOC C.
- **Pure Rust**: `image-webp` crate exists; lossless encode may
  not be complete.
- **LimniFS win**: Smaller than JXL but easier to port.
- **Action**: lower priority than JXL.

#### **QOI — Quite OK Image format** (Dominic Szablewski, MIT, 2021)

- **What**: Tiny, fast lossless image codec. ~1000 LOC. Designed
  for game engines; not as good ratio as PNG but extremely fast
  decode.
- **Status**: Stable since 2021. Multiple Rust implementations.
- **Pure Rust**: `qoi` crate, MIT.
- **LimniFS win**: Niche — game asset pipelines. Probably not
  worth integrating as a content-class codec.
- **Action**: skip unless game-asset users materialize.

#### **AVIF lossless** (AOM, BSD-3)

- **What**: AV1 Image File Format. Lossless mode uses AV1's intra
  frame tools. Comparable to JXL on natural images.
- **Status**: Mature; `libavif`.
- **Pure Rust**: Not available.
- **LimniFS win**: Same as JXL.
- **Action**: skip — JXL is a better long-term bet (royalty-free
  IP situation clearer).

## Text-specific codecs

### Already integrated

- **FSST** (omnizip-fsst) — string-table preprocessor, composited
  with Brotli for CSV/JSON.

### Worth investigating

#### **PPMd** (Dmitry Shkarin)

- **What**: Context-tree-weighting PPM (Prediction by Partial
  Matching) codec. Best-in-class ratio on natural language text.
  Used in 7-Zip and RAR.
- **Reference impl**: LGPL-2.1 C code.
- **Spec availability**: The algorithm is fully described in
  Shkarin's paper "PPM: one step to practicality" (DCC 2001) and
  Cleary & Witten's original PPM paper (1984). The model
  structures, escape mechanisms, and probability update rules
  are all published in academic literature.
- **Clean-room status**: **VIABLE**. A from-paper reimplementation
  in Rust is independently licensed. The algorithm has no patents
  (original IBM patents expired ~2010; Shkarin's extensions were
  never patented).
- **LimniFS win**: PPMd typically beats Brotli q11 on natural
  language text by 5-15%. It's slower (10-50× Brotli's encode
  time) but for archival mode (`--codec-map=archival`) the ratio
  gain is worth it.
- **Action**: **File omnizip issue.** Mark as "clean-room from
  DCC 2001 paper."

#### **bzip2** (Julian Seward)

- **What**: Burrows-Wheeler Transform + Move-To-Front + RLE +
  Huffman. Classic codec.
- **Status**: Mature; reference impl is BSD-3 (no license issue
  at all — was incorrectly dismissed earlier).
- **Pure Rust**: `bzip2-rs` crate exists (MIT), pure Rust, no
  C deps. Ready to use today.
- **LimniFS win**: Small on ratio (loses to Brotli/LZMA) but
  has **universal interop** — every Linux system can decode
  bzip2. Useful as a fallback codec for compatibility with
  legacy systems.
- **Action**: low priority, but NOT dismissed. Wire if
  compatibility with legacy tools becomes a requirement.

#### **DictTar / Pre-dicted Tar** (Kanghua Dai et al., 2024)

- **What**: Research codec that applies a pre-trained dictionary
  to tar-like streams. Designed for package-manager workloads
  (npm, pip).
- **Status**: Paper at USENIX ATC 2024. Reference impl in C.
- **LimniFS win**: Could be very strong for the npm/pip use case
  (lots of small text files with shared boilerplate).
- **Pure Rust**: Not available.
- **Action**: watch. If productionized and ports to Rust, worth
  considering for the "package archive" scenario.

## Entropy-coding primitives

### Already integrated

- **Huffman** (omnizip-zstd/huffman, package-merge length-limited)

### Worth investigating

#### **rANS (range Asymmetric Numeral Systems)**

- **What**: Alternative to Huffman that handles skewed distributions
  better. ~3× faster than arithmetic coding, 5-10% better than
  Huffman on text.
- **Status**: Used in JPEG XL, DAALA, BitKnit. Multiple Rust impls.
- **Pure Rust**: `rans-rs`, `constriction`.
- **LimniFS win**: Standalone — small. As a component of a larger
  codec (replacing Huffman in ZSTD) — captured already.
- **Action**: skip standalone.

#### **Arithmetic coding with adaptive models**

- **What**: Optimal entropy coding. Used in JPEG XR, H.264.
- **Status**: Slow. Patents expired (last ones in 2020).
- **Pure Rust**: Multiple implementations.
- **LimniFS win**: Small standalone. Too slow vs rANS.
- **Action**: skip.

## Filters / preprocessors

### Already integrated (omnizip-filters)

- **BCJ-x86** — branch-call-jump filter for x86 binaries.
- **Delta** — byte-wise difference for sensor data.

### Worth investigating

#### **Additional BCJ filters** (omnizip TODO 65)

- PPC, IA64, ARM, ARM-Thumb, SPARC, ARM64
- Already documented in omnizip's TODO.complete. Will ship when
  omnizip picks them up.

#### **ETC1/ETC2 image delta** (Khronos)

- **What**: Predictive filter for compressed GPU textures.
- **Status**: Niche.
- **Action**: skip.

#### **String-table preprocessors beyond FSST**

- **What**: Multiple academic string-dedup approaches (SATO,
  StringCompress).
- **Action**: FSST is the industry standard and already integrated.
  Others offer marginal gains.

## What LimniFS should NOT do

- **Neural codecs** — non-deterministic across hardware. Even with
  fixed model weights, floating-point operation order varies by
  SIMD width / GPU architecture, producing different bit streams.
  Violates `DropId = BLAKE3(plaintext)` invariant. This is a
  fundamental property of the approach, not a license issue.
- **Lossy codecs** (MPC, Opus lossy, JPEG, AV1 lossy) — LimniFS
  is lossless-only by design.
- **Proprietary codecs** (TAK) — closed-source, Windows-only
  encoder. No spec available for clean-room reimplementation.
- **Database-column codecs** (PGLZ) — designed for ~2 KB in-row
  records, not file archives. Wrong problem domain.

### Important correction: GPL/LGPL reference impls

GPL/LGPL on the **reference implementation** does NOT prevent
clean-room reimplementation from the **published spec/paper**.
Algorithms are not copyrightable; only specific code is. omnizip
already follows this pattern (ports from Ruby refs → Rust). The
same approach applies to:

- **ZPAQ** — spec is public domain. Rust impl from spec = MIT.
- **GLZA** — algorithm in published paper. Rust impl from paper = MIT.
- **PPMd** — algorithm in DCC 2001 paper. Rust impl from paper = MIT.
- **bzip2** — reference impl is BSD-3. No issue at all.

These were **incorrectly dismissed** in an earlier version of this
doc on GPL grounds. The dismissal is corrected above. The real
constraint is: implement from the spec, not from the GPL source.

## Priority queue for next omnizip round

Based on ratio wins × integration cost:

| Rank | Codec | LimniFS win | Integration cost |
|---|---|---|---|
| 1 | **FLAC LPC encoder** (omnizip TODO 62) | WAV/AIFF goes from 100% to ~20% | Low — wiring in place, just flip flag |
| 2 | **LZMA optimal parser** (omnizip TODO 64) | LZMA could finally beat ZSTD on text | Low — already wired |
| 3 | **ZSTD dictionaries** | tiny-files 69% → ~25% | Medium — needs dict training + loader |
| 4 | **ZPAQ** (clean-room from public-domain spec) | Best archival ratio; beats LZMA by 10-20% | Medium — ~3000 LOC (VM + models + container) |
| 5 | **JPEG XL lossless** | PNG archives -20% | High — port encoder or use FFI |
| 6 | **BLOSC2** | FITS/scientific -40% on floats | High — port container + shuffle |
| 7 | **PPMd** (clean-room from DCC 2001 paper) | Text ratio beats Brotli by 5-15% | Medium — ~2000 LOC |
| 8 | **GLZA** (clean-room from spec) | DNA/logs with hierarchical repetition | Medium — ~3000 LOC |
| 9 | **WebP lossless** | PNG archives -10% | Medium — pure Rust encoder incomplete |
| 10 | **bzip2** (already pure Rust) | Universal interop with legacy tools | Low — crate exists |

Items 1-2 are already on omnizip's TODO. Items 3-6 need new
proposals.

## Concrete next steps

1. **File omnizip issue** for ZSTD dictionary mode — small ask,
   high ROI on tiny-files.
2. **File omnizip issue** for BLOSC2 (shuffle + container format)
   — bigger ask, would unlock scientific data workloads.
3. **Local port**: jxl-oxide has a Rust decoder; check if the
   encoder is feasible to start (would unlock PNG archives).
4. **Watch**: DictTar paper — if productionized, file omnizip issue.

## References

- DwarFS source: `src/external/dwarfs-t/`
- omnizip-rs: `~/src/omnizip/omnizip-rs/`
- Existing proposals:
  - `docs/omnizip-rs-proposal.md`
  - `docs/omnizip-0.4-followups.md`
  - `docs/omnizip-0.5-followups.md`
  - `docs/omnizip-vs-limnifs-boundary.md`
  - `docs/dwarfs-multicodec-investigation.md`
- Survey papers (2024):
  - "Comprehensive Evaluation of Lossless Compression for Scientific
    Data" — ICDE 2024 (BLOSC comparison)
  - "Neural Compression in 2024: A Survey" — non-deterministic,
    skip for LimniFS
  - "Lossless Image Compression with JPEG XL" — DCC 2023
