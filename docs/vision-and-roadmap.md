# LimniFS — Vision, Differentiators, and Roadmap

## What makes LimniFS special

LimniFS is not "another compressed filesystem." It is the first image
format designed from the ground up for a world where **content is
addressed, builds are reproducible, and trust is verified — not
assumed.**

Seven properties that NO competing format has ALL of:

### 1. Content-addressed identity

`DropId = BLAKE3(plaintext)`. Identity is the hash of the content —
not a block number, not an inode, not a filename. This means:

- Identical files deduplicate across images, automatically, without
  any explicit delta logic. Two Linux kernel tarballs share 95%+ of
  their drops.
- Re-encoding with a different codec does NOT change identity. The
  same file compressed with LZ4 today and ZSTD tomorrow has the same
  DropId — the codec is a representation, not identity.
- Integrity verification recomputes BLAKE3 and compares. It does not
  trust metadata. A corrupted metadata block cannot lie about content
  integrity because the hash is of the plaintext, not the stored bytes.

No other filesystem image format works this way. SquashFS, DwarFS,
tar, and cpio all use positional identity. Git and OSTree use content
addressing but are not filesystem image formats.

### 2. Pure Rust, zero C dependencies

The reference implementation IS Rust. Not a C library with Rust
bindings. Not a Rust wrapper around libzstd. The entire stack —
BLAKE3, LZ4, ZSTD decode, Brotli, DEFLATE, Snappy, XZ decode,
XChaCha20-Poly1305, Ed25519, X25519, Reed-Solomon — is pure Rust with
`#![forbid(unsafe_code)]`.

This matters because:

- **Air-gapped builds:** compile on a machine with no C toolchain, no
  system headers, no pkg-config. `cargo build` is the entire build.
- **Reproducible builds:** every dependency is a Rust crate with a
  pinned version in Cargo.lock. No system library version drift.
- **Supply chain security:** no C code means no buffer overflows in
  the codec layer. The `unsafe` audit surface is zero.
- **Cross-compilation:** `cargo build --target aarch64-unknown-linux-musl`
  just works. No C cross-compiler needed.

### 3. Per-content-class compression

The seine classifier inspects each chunk and routes it to the optimal
codec:

- **Text/Code** → ZSTD (fast, good ratio on natural language)
- **Binary structured** → LZ4 (fast, handles redundancy well)
- **Binary random** → Store (incompressible; skip the CPU)
- **Already compressed** → Store (media, archives; double compression
  wastes CPU)

Most formats pick ONE codec for the whole image. LimniFS adapts per
chunk. A single image might have text compressed with ZSTD at 3:1,
binaries with LZ4 at 2:1, and media stored uncompressed — all in the
same file, all transparently.

### 4. Feature-flagged sovereignty

The default build (`cargo build`) has:

- Zero network code
- Zero C code
- Zero system dependencies
- Zero attack surface beyond BLAKE3 and the codecs

Every capability that touches the outside world is behind a feature
flag: `http` (locators), `fuse` (mount), `key-wrap` (HPKE), `signing`
(Ed25519). Users opt in to exactly the surface they need.

This is the opposite of most tools, where the default build pulls in
everything and users must configure their way to a minimal install.

### 5. Remote-native

Drops can live anywhere. The locator registry knows how to fetch from:

- Local files (`FileLocator`)
- HTTP range requests (`HttpLocator` — hand-rolled HTTP/1.1, no
  external HTTP crate)
- S3 path-style (`S3Locator`)
- IPFS gateways + CAR files (`IpfsLocator`)

A `.lim` image's manifest is small (~24 KB for a 440 MB image). The
slabs (the bulk of the data) can be distributed across CDNs, S3
buckets, or IPFS nodes. The reader fetches on demand — no need to
download the entire image to access one file.

### 6. Erasure-coded

Reed-Solomon systematic Vandermonde coding per slab. Choose (k+m):
k data shards + m parity shards. Lose up to m shards and reconstruct
losslessly. This is mathematical fault tolerance — no backups, no
replication, just parity.

For archival cold storage (tape, glacier), this is the difference
between "your data is gone" and "your data is recoverable."

### 7. OCP codec registry

Adding a codec is one new file + one `register()` call. The dispatch
code never changes. The registry is a `Vec<Box<dyn Codec>>` with
runtime lookup — not a `match` statement that must be edited for every
new codec.

This means third-party codecs (a custom domain-specific compressor, a
future neural-network codec, a hardware-accelerated codec) can be
registered at runtime without forking LimniFS.

---

## Roadmap

### v0.2.0 — Performance & Usability (Q3 2026)

| Feature | Impact |
|---|---|
| **mmap slab reader** | Zero-copy access to compressed bytes; images larger than RAM |
| **Parallel decompression** | Multi-threaded block decode; 2-4x faster extract on multi-core |
| **ZSTD dictionaries** | 2-4x better ratio on small files (<100 KB); critical for node_modules/gems |
| **Streaming create** | Chunk → hash → compress → write incrementally; no memory limit on image size |
| **Delta images** | `base.lim` + `delta.lim` share drops; incremental backup/distribution |
| **Shell completions** | `limni completions bash/zsh/fish` |
| **JSON output** | `--json` on every command for scripting/CI |
| **Progress bars** | Visual feedback for large create/extract |
| **Dry-run mode** | `limni limn --dry-run` shows predicted ratios without writing |
| **`limni convert`** | Convert tar.gz / .squashfs / .dwarfs → .lim |

### v0.3.0 — Writable Images (Q4 2026)

| Feature | Impact |
|---|---|
| **Epoch replay** | Apply operations from epoch chain to reconstruct state at any point |
| **Epoch commit** | `limni commit overlay/ base.lim` diffs and produces a new epoch file |
| **Overlay mount** | Mount multiple `.lim` images as layers (container image model) |
| **Time-travel mount** | `limni mount --epoch 5 image.lim /mnt` mounts state at epoch 5 |
| **Branching** | Fork an epoch chain; merge via CRDT or 3-way diff |
| **Diff viewer** | `limni diff epoch-3 epoch-7` shows changes across epochs |
| **Persistent tree** | On-disk index for O(1) path lookup without re-parsing metadata |

### v0.4.0 — Container & Distribution (Q1 2027)

| Feature | Impact |
|---|---|
| **OCI layer adapter** | `.lim` as an OCI image layer; `docker pull` works |
| **Composefs/EROFS export** | Kernel-level mount via fs-verity; zero-copy kernel path |
| **Network-first mode** | Manifest-only `.lim`; drops fetched on-demand from locators |
| **Deduplication server** | Shared drop store for multiple LimniFS instances |
| **Compression profiles** | `--profile web`, `--profile archive`, `--profile embedded` |
| **Solid compression** | Cross-file redundancy exploitation for archival tier |

### v1.0.0 — Stability & Trust (Q2 2027)

| Feature | Impact |
|---|---|
| **Wire format frozen** | v1 spec; no breaking format changes |
| **Security audit** | Third-party audit of crypto, parser, and codec paths |
| **API stability** | Semver commitment for library consumers |
| **Full conformance suite** | 100+ test vectors covering every format edge case |
| **Benchmark parity** | Performance ≥ every major format on Silesia + enwik9 |

### v2.0.0 — Advanced (2027+)

| Feature | Impact |
|---|---|
| **WASM operations** | Programmable transformations on images (filter, transform, analyze) |
| **Blockchain anchoring** | OpenTimestamps (Bitcoin) and Ethereum manifest anchoring |
| **Post-quantum signatures** | Dilithium / Falcon manifest signing |
| **Self-healing** | Automatic repair from erasure-coded parity on detected corruption |
| **Tiered storage** | Hot/warm/cold codec migration based on access frequency |
| **Proof-of-replication** | Cryptographic proof that a server actually stores a drop |
| **Selective disclosure** | Prove a file exists in an image without revealing its contents |
| **GDPR right-to-be-forgotten** | Cryptographic erasure via key destruction |

---

## How to tell the world

### The elevator pitch

> LimniFS is a pure-Rust filesystem image format where every file's
> identity is its BLAKE3 hash, compression adapts to content type, and
> the default build has zero C dependencies. It's faster than DwarFS,
> air-gapped safe, and content-addressed end-to-end.

### The comparison that lands

| | tar.gz | SquashFS | DwarFS | **LimniFS** |
|---|---|---|---|---|
| Content-addressed | no | no | no | **yes (BLAKE3)** |
| Pure Rust | no | no | no | **yes** |
| Per-content compression | no | no | no | **yes (seine)** |
| Random access | no | yes | yes | **yes** |
| Remote-native | no | no | no | **yes (locators)** |
| Erasure coding | no | no | no | **yes (Reed-Solomon)** |
| Create speed | slow | medium | fast | **fastest** |

### The demo that converts

An interactive WASM playground where a visitor drops a folder, watches
it pack into a `.lim` image in the browser, browses the tree, sees
per-file compression ratios and BLAKE3 hashes, and downloads the image.
This is the single highest-leverage piece of content for adoption.

### The benchmark that proves

A live dashboard (CI-driven) showing LimniFS vs DwarFS vs SquashFS
vs tar.gz across Silesia, enwik9, and the Linux kernel source tree.
Create time, extract time, image size, and per-codec breakdown. Numbers
don't lie.

### The ecosystem that scales

- **tebako integration** — LimniFS as a pure-Rust OFFS adapter,
  eliminating DwarFS's C++ build complexity for Ruby/Python packaging
- **OCI adapter** — `.lim` as a container layer format
- **omnizip-rs** — pure-Rust codec library (LZMA, ZSTD, Brotli, DEFLATE,
  bzip2, PPMd) reusable beyond LimniFS
- **limnifs-py** — Python reference reader (spec-sufficiency oracle)
- **limnifs-frozen2** — DwarFS migration adapter

---

## The deeper vision

LimniFS is not just a filesystem format. It's a **content-addressed
storage primitive** that happens to present as a filesystem.

The same drop store, codec registry, and locator system that powers
`.lim` image files can power:

- **Container image layers** (OCI-compatible, content-addressed)
- **Package archives** (npm/cargo/gem replacement with dedup)
- **Backup systems** (incremental, erasure-coded, verified)
- **CDN edge storage** (content-addressed chunks, served on demand)
- **Embedded firmware** (pure-Rust, small binary, air-gapped build)
- **Scientific data** (reproducible, integrity-verified, large-scale)

The filesystem API is the user-facing surface. The content-addressed
drop store is the foundation. Everything LimniFS builds on top —
codecs, locators, crypto, epochs — is a layer on that foundation.

This is why the three-layer architecture (drop store / metadata /
manifest) matters: each layer is independently useful. The drop store
alone is a content-addressed blob store. The metadata layer alone is a
Merkle directory tree. The manifest alone is a signed root of trust.
Compose them and you get a filesystem. Decompose them and you get
building blocks for other systems.

**That's what makes LimniFS special.**
