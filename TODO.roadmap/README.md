# LimniFS Roadmap

All remaining work, organized by priority. Each file is a self-contained
task spec with goals, acceptance criteria, and dependency links.

## Air-gapped baseline

The default build (`cargo build`) MUST work on air-gapped machines with
no network, no C libraries, no external services. Feature flags opt in
to capabilities that require external resources:

| Flag | Adds | Air-gapped safe? |
|---|---|---|
| (none, default) | BLAKE3, LZ4, Ed25519, local timestamps | ✅ |
| `zstd` | Zstandard compression (libzstd) | ⚠️ C dep |
| `xz` | XZ/LZMA2 compression (liblzma) | ⚠️ C dep |
| `http` | HTTP/S3/IPFS locators | ⚠️ Network |
| `fuse` | FUSE mount | ⚠️ System FUSE |
| `key-wrap` | HPKE X25519 key wrap | ✅ Pure Rust |
| `signing` | Ed25519 manifest signing | ✅ Pure Rust |
| `wasm-ops` | WASM programmable operations | ✅ Pure Rust |
| `anchor-ots` | OpenTimestamps blockchain anchoring | ❌ Bitcoin RPC |
| `anchor-eth` | Ethereum anchoring | ❌ Ethereum RPC |
| `p2p` | libp2p epoch gossip | ❌ Network |

## Priority order

| # | File | Phase | Priority | Depends on |
|---|---|---|---|---|
| 01 | 01-feature-gate-codecs.md | Foundation | P0 (blocks air-gapped) | — |
| 02 | 02-epoch-format.md | Writable | P0 | — |
| 03 | 03-epoch-replay.md | Writable | P0 | 02 |
| 04 | 04-epoch-commit.md | Writable | P0 | 02, 03 |
| 05 | 05-overlay-mount.md | Writable | P1 | 02, fuse |
| 06 | 06-codec-map-flag.md | Codec | P1 | 01 |
| 07 | 07-enhanced-classifier.md | Codec | P1 | — |
| 08 | 08-epoch-signatures.md | Provenance | P1 | 02, signing |
| 09 | 09-epoch-timestamps.md | Provenance | P1 | 02 |
| 10 | 10-policy-operations.md | Provenance | P1 | 02 |
| 11 | 11-seal-operation.md | Compliance | P1 | 02, 10 |
| 12 | 12-semantic-operations.md | Provenance | P2 | 02 |
| 13 | 13-wasm-operations.md | Provenance | P2 | 02, wasm-ops |
| 14 | 14-crdt-merge.md | Distributed | P2 | 02 |
| 15 | 15-persistent-tree.md | Performance | P2 | 02 |
| 16 | 16-accumulator-proofs.md | Performance | P2 | 02, 15 |
| 17 | 17-parallel-replay.md | Performance | P2 | 03 |
| 18 | 18-branching.md | DX | P2 | 02 |
| 19 | 19-diff-epoch.md | DX | P2 | 02 |
| 20 | 20-time-travel-mount.md | DX | P2 | 05, 15 |
| 21 | 21-epoch-streaming.md | Distribution | P3 | 02 |
| 22 | 22-blockchain-anchoring.md | Distribution | P3 | 02, 08 |
| 23 | 23-p2p-distribution.md | Distribution | P3 | 02 |
| 24 | 24-zk-verification.md | Security | P3 | 02 |
| 25 | 25-post-quantum.md | Security | P3 | 08 |
| 26 | 26-proof-of-replication.md | Security | P3 | 02 |
| 27 | 27-selective-disclosure.md | Privacy | P3 | 02, 24 |
| 28 | 28-gdpr-forget.md | Compliance | P3 | 02, 10, 11 |
| 29 | 29-self-healing.md | Reliability | P3 | 02 |
| 30 | 30-tiered-storage.md | Performance | P3 | 02 |
