# Proposal: what LimniFS needs from omnizip-rs

This is a downstream consumer's wishlist, written after integrating
omnizip-rs 0.2 / 0.3 into LimniFS and benchmarking the result against
SquashFS, DwarFS, and tar+zstd on real datasets (PHP source, Python
source, AI model weights, synthetic stress tests).

The intent is to make the next phases of omnizip-rs concrete: which
gaps close which competitive gaps, in what order, with what API.

## Where we are now

LimniFS uses these omnizip codecs today:

| Codec | omn-zip module | Encode | Decode | Notes |
|---|---|---|---|---|
| store 0x00 | (none) | ✅ trivial | ✅ trivial | — |
| lz4 0x01 | omnizip-lz4 (lz4_flex wrap) | ✅ | ✅ | fast, low ratio |
| zstd 0x02 | ruzstd (NOT omnizip) | ✅ L1 (weak) | ✅ | ruzstd encode is barely functional |
| xz 0x03 | omnizip-lzma | ✅ (Phase B literal-only) | ✅ | encoder emits one literal per byte |
| brotli 0x04 | brotli crate (NOT omnizip) | ✅ q0–q11 | ✅ | workhorse for source code |
| deflate 0x05 | miniz_oxide wrap | ✅ | ✅ | — |
| snappy 0x06 | omnizip-snappy | ✅ | ✅ | — |

So today only **brotli** is doing real compression work on text/code
content. The ZSTD slot is held back by ruzstd; the XZ slot has an
encoder that produces output larger than its input.

## Benchmark picture (PHP source, 140 MB, 22 K inodes)

After LimniFS-side optimisations (metadata compression, per-file
dedup, sparse routing, quality scaling):

| Format | Create (s) | Ratio (%) | Encoder |
|---|---:|---:|---|
| **LimniFS (Brotli q5)** | 1.09 | **13.53** | brotli crate q5 per chunk + q2 on metadata |
| SquashFS | 0.43 | 14.41 | libzstd L1 (C) |
| tar+zstd | 1.00 | 14.53 | libzstd L1 (C) |
| DwarFS | 1.10 | 20.72 | liblzma (C) |

LimniFS **already wins on ratio** with Brotli. We **lose on speed**
because Brotli q5 is ~3× slower than libzstd L1 per byte.

## What we need from omnizip

### P0 — Real ZSTD encoder (closes the speed gap)

**Why:** libzstd L1 is the SquashFS/tar+zstd encoder. At ZSTD L1 we
would match SquashFS's speed AND beat its ratio (Brotli q5 already
beats its ratio at L1). At ZSTD L3 / L6 we would also beat DwarFS's
ratio.

**Target:**

| Level | Encode throughput | Ratio on source code | Use case |
|---|---:|---:|---|
| L1 (Fastest) | ≥ 300 MB/s | ≤ 16% | default; competitive with libzstd L1 |
| L3 (Fast) | ≥ 200 MB/s | ≤ 14% | balanced |
| L6 (Default) | ≥ 100 MB/s | ≤ 12% | "best ratio" mode |
| L19+ (Best) | any | ≤ 10% | archival |

**API requirements:**

1. `pub fn encode_frame(plaintext: &[u8], level: ZstdLevel) -> Result<Vec<u8>, ZstdError>`
   — already exists; needs Phase C body (real Huffman + sequences +
   match finder).
2. **Deterministic output.** Same input + level → byte-identical
   output across runs, versions, and hosts. This is non-negotiable for
   LimniFS: DropIds are BLAKE3 of plaintext, but image reproduction
   requires the slab bytes to be deterministic too.
3. **Single-segment frames with explicit FCS.** The current Phase B
   encoder already does this; preserve it.
4. **No content size limit.** LimniFS slabs are ≤ 64 MiB but a single
   drop can be any size the caller hands in.
5. **No malloc of huge intermediate buffers.** Encode should work
   in-place on streaming input where possible.

**Out of scope for LimniFS:** dictionary mode (we use FastCDC chunks
at 64 KiB–1 MiB; dictionaries help small files but LimniFS routes
those inline, not through ZSTD).

**Estimated impact on LimniFS PHP benchmark:** create 1.09s → 0.4–0.6s
(matching SquashFS), ratio 13.5% → 11–13% (matching or beating
SquashFS).

### P0 — Real LZMA encoder (closes the ratio gap vs DwarFS)

**Why:** DwarFS uses LZMA to get 20.7% on PHP source. With a working
LZMA encoder we could match DwarFS's compression strategy exactly.

**Target:**

| Level | Encode throughput | Ratio on source code | Use case |
|---|---:|---:|---|
| 0 | ≥ 100 MB/s | ≤ 20% | fast archival |
| 6 (default) | ≥ 20 MB/s | ≤ 16% | balanced |
| 9 | any | ≤ 14% | max ratio |

**API requirements:**

1. `pub fn xz_compress(plaintext: &[u8], level: LzmaLevel) -> Result<Vec<u8>, LzmaError>`
   — already exists but takes no level. Add a level parameter.
2. Real LZMA2 encoding inside the XZ container. Today
   `encode_lzma2_stream` calls `Lzma1Encoder::encode` which is Phase B
   (literal-only). Phase C needs the match finder wired in — the
   `encoder/match_finder.rs` file exists; just integrate it.
3. **Determinism** — same constraint as ZSTD.
4. Deterministic filter chain. v0.1 of LimniFS uses no filters
   (delta/BCJ); if omnizip adds filters, LimniFS will opt in
   separately per content class.

**Estimated impact on LimniFS PHP benchmark:** at LZMA-6, ratio 13.5%
→ ~14% (slightly worse than current Brotli q5 but with much better
interoperability — LZMA is the industry-standard archival codec). At
LZMA-9 with a delta filter on binary chunks, could reach ~11%.

### P1 — Streaming encoder API

**Why:** LimniFS slabs are bounded at 64 MiB today, but the metadata
blob (post-compression) and future "solid windows" (multiple files
compressed together) can be 100+ MiB. The current `encode_frame`
takes `&[u8]` and allocates the whole input in memory. A streaming
variant lets us compress on the way to disk without buffering.

**Target API:**

```rust
pub struct ZstdEncoder { /* ... */ }
impl ZstdEncoder {
    pub fn new(level: ZstdLevel) -> Self;
    pub fn write_chunk(&mut self, bytes: &[u8]) -> Result<()>;
    pub fn finish(self) -> Result<Vec<u8>>;
}
```

Same shape for LZMA. The decoder side already streams (good).

### P2 — Quality/level enums exposed through the Codec trait

Today the omnizip-codecs `Codec` trait takes no level parameter:

```rust
pub trait Codec {
    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, OmnizipError>;
    // ...
}
```

Each codec picks a hardcoded default. That's fine for round-tripping
but loses the level-axis tuning that real users want. Two options:

**A.** Add `compress_with_level(plaintext, level: CompressionLevel)` to
the trait (default-impl delegates to `compress`).

**B.** Register multiple instances of each codec at different levels
(`ZstdCodec::fastest()`, `ZstdCodec::default()`, etc.).

LimniFS's preferred shape is **A**, because we route by content class
at the writer layer and want to override the level per-call without
registering 22 codecs.

### P3 — Dictionary support (deferred)

Useful for tiny-file compression (node_modules-like workloads) but
not a current bottleneck for LimniFS — we route sub-4 KiB files
inline rather than through a codec. Defer until ZSTD/LZMA encoders
are at parity with the C reference.

## Acceptance criteria (per phase)

**omnizip-zstd Phase C** is "done" for LimniFS when:
- `encode_frame(input, ZstdLevel::Default)` on `enwik8` produces
  output ≤ 36.5 MB (libzstd L6 reference is 36.47 MB).
- Determinism: encoding the same input at the same level twice
  produces byte-identical output.
- LimniFS's `bench_*.md` shows LimniFS create ≤ 0.6 s on the php
  benchmark (matching or beating SquashFS).

**omnizip-lzma Phase C** is "done" for LimniFS when:
- `xz_compress(input, LzmaLevel::default())` on `enwik8` produces
  output ≤ 27 MB (liblzma L6 reference is 26.9 MB).
- `xz_compress(input, LzmaLevel::best())` on `enwik8` produces
  output ≤ 23 MB (liblzma L9 reference is 22.9 MB).
- Determinism as above.

## What LimniFS will do on its side (no omnizip work needed)

These are in flight or done, listed so omnizip maintainers can see
what the consumer is doing to pull its own weight:

- ✅ Metadata blob compression (Brotli, quality-scaled by blob size)
- ✅ Per-file FastCDC dedup (skip compression for duplicate chunks
  within one file)
- ✅ Sparse-class routing to Brotli (zeros compress 100×)
- ✅ Quality-scaling for large blobs (q2 for >256 KiB metadata)
- ✅ Drop-packing correctness fix (fall back to STORE when compression
  produces larger output — handles Phase B encoders cleanly)
- 🚧 Solid compression for tiny inline files (would close the
  tiny-files ratio gap vs SquashFS without needing a better codec)
- 🚧 Compact inode encoding (varint fields, dedup repeated modes —
  wire-format change)

## Suggested phase order for omnizip-rs

If only one of (ZSTD Phase C, LZMA Phase C) lands first, **ZSTD Phase
C** is the higher-leverage one for LimniFS:

- ZSTD L6 closes BOTH the speed gap (vs SquashFS) and the ratio gap
  (we'd actually beat SquashFS).
- LZMA L6 only closes the ratio gap to DwarFS, but Brotli q5 already
  does that.
- LZMA is also a more complex port (match finder + range coder
  integration); ZSTD Phase C mostly wires together existing FSE +
  Huffman + sequences pieces.

Recommended order: **ZSTD Phase C → LZMA Phase C → streaming APIs
→ dictionary support.**

## References

- LimniFS benchmark report (latest): `benchmarks/results/bench_*.md`
- LimniFS session 32 STATUS entry: `TODO.impl/STATUS.md` (top of file)
- omnizip-zstd BUGREPORT (open decoder bugs, if any remain after
  0.3): `BUGREPORT-zstd-0.1.0.md` at omnizip-rs root
