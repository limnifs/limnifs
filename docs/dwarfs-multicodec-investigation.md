# Investigation: DwarFS multi-codec strategy vs LimniFS

Source: `src/external/dwarfs-t/` (DwarFS v0.14.1 + the README's
"Specialized Algorithms" section).

## What DwarFS actually does

DwarFS runs **5 categorizers** on every file *before* chunking. Each
categorizer can claim all or part of a file for a category, and each
category maps to a specific compressor with category-specific
parameters extracted from the file's header.

```
src/writer/categorizer/
├── binary_categorizer.cpp       ELF/Mach-O/PE  →  binary/{osabi}
├── fits_categorizer.cpp         FITS images    →  fits/image
├── pcmaudio_categorizer.cpp     WAV/AIFF PCM   →  pcmaudio/waveform
├── incompressible_categorizer.cpp  high-entropy →  incompressible
└── libmagic_categorizer.cpp     libmagic db    →  arbitrary categories
```

The categories drive codec selection. DwarFS's compressor registry
(`src/compression/`):

| Codec | Use case | Ratio | Speed |
|---|---|---:|---:|
| **null** | incompressible / already-compressed | 0% | 4 GB/s |
| **lz4 / lz4hc** | fast, low ratio | 89–96% | 350–1900 MB/s |
| **zstd** | default balanced | 99% | 2400 MB/s |
| **brotli** | web content | 99% | 421 MB/s |
| **lzma** | max ratio | 48% (i.e. 52% saved) | 8 MB/s |
| **flac** | **PCM audio** (WAV, AIFF) | **83% saved** | 188 MB/s |
| **ricepp** | **FITS astronomical images** | **68% saved** | 359 MB/s |

Plus **FSST** (`src/internal/fsst.cpp`) — a *preprocessor* that finds
the most common substrings in a block and replaces each with a single
byte. Runs *before* the main codec. Helps text-heavy and
repetitive-data workloads.

### The two specialized codecs we don't have

**FLAC** (`src/compression/flac.cpp`):
- Input: raw PCM audio samples (16/24/32-bit, any sample rate, mono
  or multi-channel).
- Algorithm: linear prediction → residual coding → Rice coding of
  residuals.
- Why it beats zstd/lzma on audio: those codecs see audio as
  byte-strings; FLAC sees them as waveforms. Linear prediction
  captures the sample-to-sample correlation that byte-oriented
  codecs miss.
- DwarFS result: **83% saved on PCM audio** vs ~30–40% for zstd L3.

**Rice++** (`src/compression/ricepp.cpp`, plus external `ricepp/`
library):
- Input: 2D image data with explicit `bytes_per_sample`,
  `component_count`, `unused_lsb_count`, endianness — i.e. FITS
  pixels, raw sensor data, scientific images.
- Algorithm: Rice coding of residuals (similar to FLAC's residual
  stage but without prediction; tuned for low-entropy positive
  integers like astronomical pixel values).
- DwarFS result: **68% saved on FITS** vs ~40% for lzma.

### How DwarFS wires it together

```
mkdwarfs --categorize      ← default-on auto-detection
mkdwarfs --compression=flac:level=3
mkdwarfs --compression=ricepp:block_size=128
```

The categorizers extract codec-specific metadata from the file
header (e.g. PCM sample format from WAV; bit depth from FITS) and
pass it to the codec so the codec doesn't have to re-detect.

## What LimniFS does today

```
file → walk → if size > INLINE_THRESHOLD:
         FastCDC chunks (256 KiB avg)
         per-chunk: classify (entropy + magic bytes) → codec
       else: inline in inode
```

Our classifier (`limnifs-write/src/classifier.rs`) has 6 classes:

| Class | Detection | Routes to |
|---|---|---|
| `Text` | entropy 0.5–7.5 + printable ≥ 85% | Brotli q5 |
| `Code` | ELF/Mach-O/PE magic | Brotli q5 |
| `Binary` | fallback | LZ4 |
| `Compressed` | entropy ≥ 7.5 OR gzip/zstd/xz magic | STORE |
| `Media` | JPEG/PNG/GIF/WebP/MP3/FLAC/MP4/Ogg magic | STORE |
| `Sparse` | entropy < 0.5 + zero ratio ≥ 80% | Brotli q5 |

That's **3 codecs** doing real work (Brotli, LZ4, STORE). For
everything we call "Media" we use STORE — which is correct for
already-compressed JPEG/PNG/MP4 but **wrong for the uncompressed
formats we don't even detect**: WAV, AIFF, FITS, BMP, raw TIFF.

## The gap

| Workload | DwarFS | LimniFS | Gap |
|---|---|---|---|
| Source code (PHP, Python) | zstd/lzma | Brotli q5 | already competitive |
| PCM audio (WAV, AIFF) | **flac** — 83% saved | LZ4 — ~30% saved (Binary class) | **2.5× worse** |
| FITS astronomical images | **ricepp** — 68% saved | LZ4 — ~40% saved (Binary class) | **1.7× worse** |
| JPEG/PNG/MP4 (compressed) | null (store) | STORE | tied ✅ |
| Random/encrypted | incompressible_categorizer → null | Compressed class → STORE | tied ✅ |
| Repetitive text | FSST + brotli | Brotli q5 (we dedup within file) | competitive |
| String-table-heavy data (CSV, JSON) | FSST preprocessor | Brotli q5 | **missed optimisation** |

Three concrete things DwarFS does that we don't:

1. **Specialized codecs for specific data types** (FLAC for audio,
   ricepp for FITS, FSST for strings).
2. **File-level categorization** before chunking. We chunk first,
   losing the file-level signal (a FITS image chunked into 256 KiB
   blocks looks like generic binary to our classifier).
3. **Codec-specific parameter extraction** in the categorizer (PCM
   sample format, FITS bit depth) — the codec doesn't have to
   re-detect.

## Why we're not doing it

1. **We didn't have the codec inventory mapped.** FLAC, ricepp, and
   FSST are not in our registry. omnizip-rs doesn't have them
   either; the omnizip roadmap focuses on the general-purpose
   codecs (LZMA/ZSTD).
2. **Our pipeline operates on chunks**, not files. By the time our
   classifier sees data, file context is gone. A FITS header at the
   start of the file ends up in only the first chunk; subsequent
   chunks look like Binary.
3. **Our categorizer is entropy + magic bytes only.** No header
   parsing for WAV/AIFF/FITS to extract codec parameters.

## Recommendations

### Tier 1 — high-impact, low-effort

**1. Add `fsst` as a preprocessor for text/code/CSV/JSON.**

FSST is pure Rust-able (the algorithm is described in the VLDB 2020
paper). It finds common substrings and replaces each with a single
byte before Brotli sees the data. Reported 1.2–1.5× ratio improvement
on text-heavy workloads. Lives between classifier and codec:

```
text chunk → FSST preprocessor → Brotli q5
```

Wire format: add a new representation byte to the drop record
("fsst-brotli" composite codec).

**2. Detect "incompressible" explicitly and skip compression.**

We currently route Compressed class (entropy ≥ 7.5) to STORE. But
random/encrypted data with entropy in 6.5–7.5 lands in Binary → LZ4
→ wastes CPU. Add an `Incompressible` class with entropy ≥ 7.0 (no
magic match) → STORE. Mirrors DwarFS's `incompressible_categorizer`.

### Tier 2 — high-impact, medium-effort

**3. Add FLAC codec for PCM audio.**

The FLAC decoder exists in Rust (`claxon` crate, MIT/Apache). The
encoder... doesn't exist as a pure-Rust crate today. Two paths:

- (a) Wait for omnizip to add FLAC (would need a new proposal — not
  currently on their roadmap).
- (b) Port FLAC encoder from the C reference (`libFLAC`, BSD license)
  to Rust. ~3 000 LOC.

Wire format: new codec id 0x07 = FLAC. Classifier detects WAV/AIFF
magic → routes to FLAC codec with PCM parameters extracted from
header.

Estimated impact: WAV/AIFF files go from ~30% saved to ~83% saved.
Useful for music libraries, sound effect packs, podcast archives.

**4. Add ricepp codec for FITS images.**

The ricepp codec itself is small (~600 LOC). It already exists in
DwarFS as a separate library (`ricepp/`). Pure algorithm, no system
deps. License: MIT.

Wire format: new codec id 0x08 = ricepp. Classifier detects FITS
magic (`SIMPLE  =`) → routes to ricepp with bit-depth + endian
parameters extracted from FITS header.

Estimated impact: FITS files go from ~40% saved to ~68% saved.
Niche but huge for the astronomy / scientific-imaging community
(ESO, NASA, JWST all use FITS).

### Tier 3 — architectural, high-impact

**5. File-level categorization before chunking.**

Today our pipeline is `file → FastCDC → per-chunk classify`. The
right shape is `file → file-level categorize → if specialized codec
applies, compress whole file; else FastCDC + per-chunk classify`.

This requires a new writer mode that bypasses FastCDC for files
claimed by a specialized categorizer. The file becomes one drop
(content-addressed by its whole-file BLAKE3) compressed by the
specialized codec.

Pseudocode:

```rust
fn process_file(pf: &PendingFile) -> ChunkedFileResult {
    if let Some(special) = FILE_CATEGORIZERS.categorize(&pf.data) {
        // FLAC for PCM, ricepp for FITS, etc.
        let compressed = special.compress(&pf.data);
        return single_drop(pf, compressed, special.codec_id());
    }
    // Fall through to FastCDC + per-chunk classify (current path).
    fastcdc_and_classify(pf)
}
```

The trade-off: we lose CDC dedup on files that go through specialized
codecs (the whole file becomes one drop). For FITS/audio that's
fine — those files don't share content across files anyway.

**6. Multi-codec trial mode (DwarFS's `--compressor=luck`).**

Try each candidate codec on a sample, pick the smallest. Slow but
optimal per-chunk ratio. Wire as `--codec-map=trial` CLI flag.

## What to do this session

The Tier 1 items are local and high-leverage:

- **FSST preprocessor**: 1–2 days. Big win on text.
- **Incompressible class**: 1 hour. Saves CPU on random/encrypted.

Tier 2 needs omnizip or new codec ports; defer until omnizip-zstd
Phase C lands.

Tier 3 (file-level categorization) is the right architecture but is
a multi-week writer refactor. Defer until we have at least one
specialized codec worth routing to.

## References

- DwarFS source: `/Users/mulgogi/src/external/dwarfs-t/`
- DwarFS README codec summary: lines 110–141
- FLAC codec: `src/compression/flac.cpp`
- Rice++ codec: `src/compression/ricepp.cpp`
- PCM audio categorizer: `src/writer/categorizer/pcmaudio_categorizer.cpp`
- FITS categorizer: `src/writer/categorizer/fits_categorizer.cpp`
- FSST preprocessor: `src/internal/fsst.cpp`
- LimniFS classifier: `limnifs-write/src/classifier.rs`
- LimniFS writer pipeline: `limnifs-write/src/lib.rs::process_file`
