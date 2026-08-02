# Boundary: what belongs in omnizip-rs vs LimniFS

omnizip-rs is positioning itself as a comprehensive pure-Rust codec
library for downstream consumers. LimniFS is one such consumer. This
document proposes where each piece of the upcoming specialized-codec
work should live so both projects stay clean and reusable.

## The principle

**Codec = pure algorithm. Categorizer = domain knowledge.**

A codec crate should:
- Take bytes in, return bytes out.
- Have no filesystem, no path lookups, no file-type detection.
- Be useful to ANY consumer (web server, database, scientific archive),
  not just a filesystem.
- Own its own wire format and codec-specific parameter encoding.

A categorizer (filesystem layer) should:
- Decide WHICH codec to use for a given file or chunk.
- Parse file headers to extract codec parameters (e.g. WAV sample
  format → FLAC params).
- Be LimniFS-specific because the routing policy is product
  strategy, not algorithm correctness.

The test: *"If I were a database storing BLOBs, would I want this
code?"* If yes → omnizip. If no → LimniFS.

## What omnizip-rs should offer

### Codec crates (pure algorithms)

| Crate | What it is | Why it belongs in omnizip |
|---|---|---|
| `omnizip-zstd` | ZSTD encoder + decoder | General-purpose. Already exists; needs Phase C encoder. |
| `omnizip-lzma` | LZMA/LZMA2/XZ encoder + decoder | General-purpose. Already exists; needs Phase C encoder. |
| `omnizip-brotli` | Brotli encoder + decoder | Already in omnizip-rs roadmap. |
| `omnizip-lz4` | LZ4 encoder + decoder | Already exists. |
| `omnizip-deflate` | DEFLATE/zlib encoder + decoder | Already exists. |
| `omnizip-snappy` | Snappy encoder + decoder | Already exists. |
| `omnizip-filters` | BCJ, delta, etc. | Already exists. |
| **`omnizip-flac`** (new) | FLAC encoder + decoder | Pure algorithm. General-purpose: databases storing PCM, music libraries, podcast archives all want it. |
| **`omnizip-ricepp`** (new) | Rice++ encoder + decoder for integer-pixel data | Pure algorithm. Useful to anyone storing FITS / scientific imaging / uncompressed sensor data. |
| **`omnizip-fsst`** (new) | Fast Static Symbol Table preprocessor | Pure algorithm. Useful for any text-heavy or repetitive-string workload (CSV columns, JSON, log files). |

### Shared infrastructure (also omnizip)

| Crate | What it is |
|---|---|
| `omnizip-codecs` | The `Codec` trait, `CodecId`, `CompressionLevel`, error types. Existing. Extend with `compress_with_level` (P2 from the previous proposal). |
| (future) `omnizip-streaming` | Streaming encoder/decoder traits. When the P1 streaming API lands. |

### Codec metadata helpers (per-codec, optional)

Each specialized codec can ship its OWN header parser as a convenience
module, since the parser is tightly coupled to the codec's parameter
format:

```rust
// In omnizip-flac
pub mod pcm_header {
    pub struct PcmParams {
        pub sample_rate: u32,
        pub channels: u8,
        pub bits_per_sample: u8,
        pub endianness: Endianness,
    }
    pub fn parse_wav(bytes: &[u8]) -> Option<PcmParams> { ... }
    pub fn parse_aiff(bytes: &[u8]) -> Option<PcmParams> { ... }
}
```

The consumer (LimniFS) calls the parser to GET parameters, then
hands them to the encoder. The parser is in omnizip because the
parameter format is defined by the codec, not by the consumer.

Same shape for `omnizip-ricepp::fits_header`.

### Acceptance criteria for each new omnizip crate

Same bar for every codec:

1. **Pure Rust** (`#![forbid(unsafe_code)]`).
2. **Round-trips** through the C reference implementation's output
   (FLAC: libFLAC; ricepp: DwarFS's ricepp; FSST: the FSST reference).
3. **Deterministic** — same input → byte-identical output across
   runs, versions, hosts.
4. **Differential tests** against the C reference (omnizip-rs's
   existing pattern).
5. **Implements the `Codec` trait** from omnizip-codecs.
6. **No filesystem or process dependencies.**

## What LimniFS keeps

### Categorizer layer (filesystem-specific)

| Module | What it does |
|---|---|
| `limnifs-write/src/classifier.rs` | Existing. Seine chunk classifier: entropy + magic bytes → 6 classes. Adding `Incompressible` class is LimniFS work. |
| `limnifs-write/src/file_categorizer/` (new) | File-level categorizers. One file per category (fits.rs, pcm_audio.rs, csv_text.rs). Each calls the corresponding omnizip header parser, then routes the whole file to the matching codec. |
| `limnifs-write/src/file_categorizer/registry.rs` | OCP registry: new categorizer = new file + register call, dispatch untouched. |

### Routing policy

The decision tree of "which file goes to which codec" is product
strategy. LimniFS picks:

- source code → FastCDC → Brotli (chunk-level)
- WAV/AIFF → FLAC (file-level, no chunking — audio doesn't share
  content across files)
- FITS → ricepp (file-level, same reasoning)
- CSV/JSON → FSST + Brotli (file-level OR chunk-level TBD by benchmark)
- JPEG/PNG/MP3/MP4 → STORE (already compressed)
- random/encrypted → STORE (incompressible)

Other consumers will make different choices. A database storing
FITS BLOBs might not bother with the "JPEG → STORE" rule. So the
routing policy lives in LimniFS, not omnizip.

### Codec registry composition

```rust
// In LimniFS's limnifs-core/src/codec/mod.rs
fn default_registry() -> &'static CodecRegistry {
    CodecRegistry::builder()
        .register(StoreCodec)
        .register(Lz4Codec)
        .register(ZstdCodec)        // wraps omnizip-zstd
        .register(XzCodec)          // wraps omnizip-lzma
        .register(BrotliCodec)      // wraps omnizip-brotli (or brotli crate directly)
        .register(DeflateCodec)
        .register(SnappyCodec)
        .register(FlacCodec)        // wraps omnizip-flac (NEW)
        .register(RiceppCodec)      // wraps omnizip-ricepp (NEW)
        .register(FsstBrotliCodec)  // composes omnizip-fsst + omnizip-brotli (NEW)
        .build()
}
```

Each "wrapper" is a thin adapter that maps the omnizip `Codec` trait
to LimniFS's codec id space and representation byte. ~30 LOC per
codec. This is LimniFS work because the codec ID allocation is
spec-pinned (LimniFS's wire format spec, not omnizip's).

### File-format detection heuristics

The categorizer decides HOW to detect a file type. Options:
- Magic bytes (WAV: `RIFF...WAVE`)
- File extension (`.fits`, `.wav`)
- Content sniffing (entropy + printable ratio for CSV)

LimniFS picks the policy. For LimniFS we'll prefer magic bytes
(content-defined) over file extensions (path-defined) because the
whole point of content addressing is to not trust names.

## Why this split (and not the alternatives)

### Alternative A: everything in omnizip

omnizip would grow filesystem-specific code (file-type detection,
routing policy). Breaks the "pure algorithm" promise. Other
consumers would have to take filesystem code they don't want.

### Alternative B: everything in LimniFS

We'd duplicate work that any pure-Rust consumer needs. If tebako
or another Rust project wants FLAC, they'd have to port it again.
Wastes effort across the ecosystem.

### Alternative C: ad hoc per codec

FLAC in omnizip, FSST in LimniFS, ricepp in omnizip, etc. — picked
case by case. No principled boundary, hard to reason about, drifts
over time.

### Recommended (this proposal)

Pure algorithms in omnizip, product policy in LimniFS. Clean test:
*"useful to a non-filesystem consumer?"* splits every time.

## Concrete work split for the upcoming features

| Feature | omnizip-rs work | LimniFS work |
|---|---|---|
| Real ZSTD encoder | omnizip-zstd Phase C | Update wrapper to expose levels (already done) |
| Real LZMA encoder | omnizip-lzma Phase C | Update wrapper to expose levels |
| Streaming encoder | New API in each codec crate | Optional: switch writer to streaming for >100 MiB blobs |
| `compress_with_level` | Trait extension in omnizip-codecs | Use it for per-call quality (already done via `compress_brotli_with_quality`) |
| **FLAC for PCM audio** | **omnizip-flac** (new crate): encoder + decoder + `pcm_header::{wav, aiff}` parsers | `limnifs-write::file_categorizer::pcm_audio`: detection + routing. `limnifs-core::codec::flac`: codec id 0x07 wrapper. |
| **Rice++ for FITS** | **omnizip-ricepp** (new crate): encoder + decoder + `fits_header` parser | `limnifs-write::file_categorizer::fits`: detection + routing. `limnifs-core::codec::ricepp`: codec id 0x08 wrapper. |
| **FSST preprocessor** | **omnizip-fsst** (new crate): encoder + decoder | `limnifs-write::file_categorizer::csv_text`: heuristic for when FSST helps. `limnifs-core::codec::fsst_brotli`: composite codec id 0x09. |
| Incompressible class | (nothing — pure heuristic) | `limnifs-write::classifier::Class::Incompressible` |
| File-level categorization framework | (nothing — architecture) | `limnifs-write::file_categorizer` module + registry |

## Suggested naming and ID allocation

**omnizip-rs crate names** (consistent with existing convention):
- `omnizip-flac`
- `omnizip-ricepp`
- `omnizip-fsst`

**LimniFS codec ids** (wire-format allocation, LimniFS's spec):
- 0x00 — store (existing)
- 0x01 — lz4 (existing)
- 0x02 — zstd (existing)
- 0x03 — xz (existing)
- 0x04 — brotli (existing)
- 0x05 — deflate (existing)
- 0x06 — snappy (existing)
- **0x07 — flac** (new)
- **0x08 — ricepp** (new)
- **0x09 — fsst+brotli composite** (new)

The composite codec (FSST+Brotli) lives in LimniFS because the
*composition* is product policy. omnizip-fsst ships the FSST
algorithm; omnizip-brotli ships Brotli; LimniFS decides to chain
them and picks the codec id.

## Test surface

**omnizip-rs side** (per codec):
- Round-trip tests.
- Differential tests vs the C reference.
- Determinism test (encode twice, assert equal).
- Property tests (quickcheck round-trip on random input).

**LimniFS side** (per categorizer):
- File-type detection tests (positive + negative samples).
- Routing tests (FITS file → ricepp codec; WAV file → flac codec).
- End-to-end benchmark on a real corpus.
- Conformance vector per new codec in `limnifs-conformance`.

## Summary

omnizip-rs offers: **pure-Rust codec crates + their header parsers +
shared traits.** That's a coherent library that any Rust consumer
can pick up.

LimniFS keeps: **categorizers, routing policy, codec id allocation,
composite codecs, registry composition.** That's product strategy
that differs per consumer.

The split is principled (the "would a database want this?" test),
consistent with how the existing omnizip-rs crates already work, and
lets the omnizip team serve multiple downstream consumers without
耦合 to filesystem-specific concerns.
