# LimniFS

**Layered, Immutable, Merkle-rooted, Network Image filesystem.**

LimniFS is a content-addressed, compressed, immutable filesystem image
format for archival, distribution, and cold storage. It packs directory
trees into a single `.lim` file with BLAKE3 content addressing,
per-content-class compression, and Merkle-rooted integrity.

## Quick start

```bash
# Build
cargo build --release

# Create an image from a directory
./target/release/limni limn my-project/ my-project.lim

# Verify integrity
./target/release/limni verify my-project.lim

# List contents
./target/release/limni ls my-project.lim

# Extract
./target/release/limni extract my-project.lim output/

# Mount (requires FUSE; Linux/macOS)
./target/release/limni --features fuse mount my-project.lim /mnt/limnifs
```

## Features

- **Content-addressed.** Every drop's identity is `BLAKE3(plaintext)`.
  Codec, encryption, and erasure coding are representations, never identity.
- **Per-content-class compression.** The seine classifier routes each
  chunk to the best codec: ZSTD for text, LZ4 for binary, store for
  incompressible data.
- **Merkle-rooted integrity.** The manifest root commits to the entire
  image. One signature verifies everything.
- **Pure Rust.** No C dependencies. Air-gapped safe. `#![forbid(unsafe_code)]`.
- **Parallel.** The writer uses rayon for parallel chunking, hashing,
  and compression.
- **Six codecs.** Store, LZ4, ZSTD, Brotli, DEFLATE, Snappy — all pure
  Rust, all behind an OCP codec registry.
- **Optional crypto.** XChaCha20-Poly1305 AEAD, HPKE key wrap, Ed25519
  signing, Shamir k-of-n secret sharing — each behind a feature flag.
- **Erasure coding.** Reed-Solomon systematic Vandermonde for
  per-slab redundancy and offline repair.
- **Remote locators.** HTTP range streaming, S3, IPFS/CAR — slabs can
  live anywhere.
- **FUSE mount.** Read-only mount on Linux and macOS.
- **Bounded random reads.** Large drops are stored as seekable
  ~256 KiB frames (zstd-seekable-style); a cold 8 KiB window decodes
  one frame, not the drop. SIEVE-evicted drop and frame caches make
  repeat windows zero-copy. See [docs/read-api.md](docs/read-api.md).

## Codec portfolio

| Id | Codec | Encode | Decode | Pure Rust |
|---|---|---|---|---|
| 0x00 | store | yes | yes | yes |
| 0x01 | LZ4 | yes | yes | yes (`lz4_flex`) |
| 0x02 | ZSTD | yes (L1) | yes | yes (`ruzstd`) |
| 0x03 | XZ/LZMA2 | yes (L6) | yes | yes (`omnizip-lzma`) |
| 0x04 | Brotli | yes (q11) | yes | yes (`brotli`) |
| 0x05 | DEFLATE | yes (L6) | yes | yes (`miniz_oxide`) |
| 0x06 | Snappy | yes | yes | yes (`omnizip-snappy` → `snap`) |

Adding a codec is one new file + one `register()` call — the dispatch
code never changes (open/closed via [`CodecRegistry`]).

## Performance

**Gated in CI.** Every claim below is a hard gate that runs on every
pull request and blocks every release (v0.3.18 readings, 2-core
GitHub runner, median of three):

| Gate | Floor | v0.3.18 reading |
|---|---:|---:|
| Warm 8 KiB random windows (`readperf`) | ≥ 200 MB/s | **13,980 MB/s** |
| Sequential extract (`readperf`) | ≥ 100 MB/s | **1,635 MB/s** |
| Create / pack throughput (`createperf`) | ≥ 50 MB/s | **209 MB/s** |
| Content hashes (`verify --deep`, E2E) | all match | green, every PR |
| BCJ-x86 on real ELF binaries | informational | 4.3% smaller than plain LZ4 |
| ZSTD dictionary win on text corpora | measured | positive, never larger |

**Why the reads are fast** — measured at v0.2.65 (Apple M-series)
and unchanged in shape since: a cold 8 KiB window into a 19.5 MiB
drop decodes ~1.03 × 256 KiB seekable frames, never the whole drop.
The failure mode behind limnifs#192 (~48 GiB of wasted decode) is
gone by construction:

| metric | monolithic | seekable | delta |
|---|---:|---:|---:|
| first 8 KiB window | 58.3 ms | 0.60 ms | **98x** |
| cold windowed | 0.1 MB/s | 11.4 MB/s | **83x** |
| warm windowed | 9099 MB/s | 6383 MB/s | 0.70x |
| sequential extract | 336 MB/s | 368 MB/s | 1.10x |
| image size | 14.52 MiB | 14.49 MiB | 1.00x |

**Versus DwarFS** (v0.2.x era, DwarFS 0.15.6, 440 MB text corpus):

| Operation | LimniFS | DwarFS | Ratio |
|---|---|---|---|
| Create | 0.62s (700 MB/s) | 1.00s | 1.6x faster |
| Extract | 0.59s (8500 files/s) | 1.37s | 2.3x faster |
| Image size | 8% smaller | baseline | — |

Codec tuning (profiles, the zstd tier map, per-codec quality knobs)
is documented in [docs/configuration.md](docs/configuration.md).

## Crate structure

| Crate | Description |
|---|---|
| [`limnifs-format`] | Wire-format primitives (header, cursor, version constants) |
| [`limnifs-core`] | Manifest parser, drop store, codec registry, crypto, locators |
| [`limnifs-write`] | Writer pipeline (FastCDC, classifier, slab packing, deepening) |
| [`limnifs-conformance`] | Test vectors and differential harness |
| [`limni`] | The `limni` CLI binary |

## Feature flags

| Flag | Adds | Air-gapped safe? |
|---|---|---|
| (default) | BLAKE3, LZ4, ZSTD, Brotli, DEFLATE, Snappy, Ed25519 | yes |
| `http` | HTTP/S3/IPFS locators | network required |
| `fuse` | FUSE mount | system FUSE required |
| `tar` | `limn --from-tar` / `limni tar` streaming (CLI) | yes |
| `sparse-index` | layer-build Bloom fast path (CLI accelerator) | yes |
| `key-wrap` | HPKE X25519 key wrap | yes |
| `signing` | Ed25519 manifest signing | yes |

Official release binaries are built with `tar` and `sparse-index`
enabled; the default `cargo build` stays minimal.

## Installation

### From source

```bash
git clone https://github.com/limnifs/limnifs.git
cd limnifs
cargo build --release
# Binary: target/release/limni
```

### From crates.io (library use)

```toml
[dependencies]
limnifs-core = "0.1"
```

## License

Apache-2.0 OR MIT.

## Links

- [Format specification](https://github.com/limnifs/spec)
- [Python reference reader](https://github.com/limnifs/limnifs-py)
- [Website](https://limnifs.org)
- [omnizip-rs](https://github.com/omnizip/omnizip-rs) — pure-Rust codec ports
