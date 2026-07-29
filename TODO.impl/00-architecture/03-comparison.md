# 03 — Comparison with extant filesystems and image formats

How LimniFS differs from what already exists, and precisely which ideas are
taken from where. Two axes matter: (1) LimniFS is an **image-format VFS** —
images live *on* filesystems/object stores, not on block devices; (2) LimniFS
treats compression, encryption, redundancy, and distribution as
**representations over a content-addressed identity plane**, not as baked-in
layout.

## 0. The classification

```
                 mutable, on block device        immutable image, on any byte store
                 ──────────────────────────      ──────────────────────────────────
local            ext4, XFS, APFS, ZFS, btrfs     SquashFS, EROFS, DwarFS, LimniFS
distributed      Ceph, GlusterFS, Taobao TFS*    OCI layers, IPFS/UnixFS, casync
                                                  composefs (hybrid: EROFS + CAS store)

* Taobao TFS is a distributed *service* with its own block layer — included
  because its slab-packing idea is adopted, but it is not an image format.
```

LimniFS competes in the top-right quadrant and borrows selectively from the
others. It is *not* a POSIX read-write filesystem, not a block allocator, not
a cluster service.

## 1. Feature matrix

| | dedup | per-class compression | staged compression | deltas/overlays | AEAD crypto | signatures | erasure coding | streaming/HTTP | spec independent of impl | license |
|---|---|---|---|---|---|---|---|---|---|---|
| **LimniFS** | ✓ BLAKE3(plaintext) | ✓ registry | ✓ tiers | ✓ 3 merge tiers | ✓ registry (4 algs) | ✓ sigstore | ✓ per-slab RS | ✓ locator registry | ✓ spec-first | Apache/MIT |
| DwarFS | ✓ | ✓ categories | ✗ build-time only | ✗ (history only) | ✗ | ✗ | ✗ | ✗ | ✗ impl-is-spec | GPL-3 |
| SquashFS | ✗ | ✗ one codec/image | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ~ (kernel docs) | tools GPL |
| EROFS | ✗ | ~ per-file lz4/zstd | ✗ | ✗ | ✗ | ✗ (fs-verity external) | ✗ | ✗ | ~ kernel UAPI | GPL-2 (kernel) |
| composefs | ✓ CAS backing | ✗ (EROFS handles) | ✗ | ✗ (overlayfs external) | ✗ | ~ fs-verity | ✗ | ~ (ostree fetch) | ~ | GPL-2/LGPL |
| OCI image layers | ~ (layer-level) | ✗ tar+gzip fixed | ✗ | ~ ordered layers | ✗ | ~ cosign external | ✗ | ✓ registry HTTP | ✓ spec | Apache |
| casync | ✓ chunks | ✗ one codec | ✗ | ✗ | ✗ | ✗ | ✗ | ✓ HTTP chunks | ~ blog+impl | LGPL |
| borg/restic | ✓ | ✗ | ✗ | ~ snapshots | ✓ | ✗ | ✗ | ~ (repo proto) | ~ | BSD/GPL |
| git | ✓ | ✗ zlib | ✗ | ✓ (packs/deltas) | ✗ | ~ commit sigs | ✗ | ✓ smart HTTP | ✓ (docs+impl) | GPL-2 |
| IPFS/UnixFS | ✓ per-block | ✗ | ✗ | ~ DAG | ✗ | ✗ | ✗ (replication) | ✓ gateways/bitswap | ✓ spec | Apache/MIT |
| Taobao TFS | n/a (service) | ✗ | ✗ | ✗ | ✗ | ✗ | ✓ replication | ✓ | ✗ | GPL-2 |
| ZFS/btrfs (contrast) | ✓ (ZFS) | ✗ one codec | ✗ | snapshots/clones | ~ (ZFS native enc) | ✗ | ✓ raidz | ✗ | ✗ | CDDL/GPL |

## 2. System-by-system: what we take, what we reject

**DwarFS / dwarfs-t** (closest relative). Take: per-class compression via
heuristics, fragment packing, strong dedup, fast random access. Reject:
build-time-only compression decisions (LimniFS deepens in the background);
implementation-as-spec monolith (LimniFS is spec-first with two readers);
GPL-3; no crypto/signatures; no deltas or locators. dwarfs-t already proved
the C++ stack can be modernized (folly/thrift removal) — LimniFS carries that
instinct into a Rust rewrite with a stable format boundary.

**SquashFS**. Take: simplicity, ubiquity, kernel mountability. Reject: one
codec per image, no dedup, no distribution story. SquashFS is the baseline
LimniFS must beat on ratio (trivial) and on mount convenience (the FUSE +
composefs paths exist for this).

**EROFS**. Take: kernel-native read performance, per-file compression choice.
Reject: kernel-coupled metadata evolution, no identity plane (fs-verity is
external), no crypto/deltas. LimniFS's composefs-style path (11) borrows
EROFS *as a component* rather than competing with it.

**composefs**. Take: the split of metadata (EROFS) from a content-addressed
object store; kernel-verified mounting. Reject: no compression pipeline of
its own, no delta semantics, no crypto in the store, Linux-only. LimniFS
generalizes the same idea to be locator-agnostic and distribution-native.

**OCI images**. Take: registry UX, content-addressed blobs, cosign-style
signing (sigstore). Reject: tar-layer granularity (ordered, poor dedup,
whole-layer pull for one file). LimniFS deltas + range streaming are the
direct answer to OCI's two worst properties.

**casync**. Take: chunk store + index separation, HTTP chunk serving. Reject:
single codec, no fs-metadata sophistication, no crypto. LimniFS's slab index
is casync's `.caibx` idea with an identity plane and a real VFS on top.

**borg/restic**. Take: authenticated encryption done right (they are the
proof the audience wants it). Reject: backup-repo semantics (mutable stores,
lock files), not mountable-first, no staged compression.

**git**. Take: delta chains, DAG versioning, content addressing as identity.
Reject: per-object granularity at storage, zlib-only, no range streaming.

**IPFS/UnixFS**. Take: multihash identity (LimniFS `DropId`s are
multihash-compatible), CAR transport, DAG pinning. Reject: per-block object
overhead (LimniFS batches into slabs — the Taobao TFS lesson applied to
IPFS-scale distribution).

**Taobao TFS**. Take: slab packing of small files into large objects with
two-level addressing — the single most important storage-layout idea outside
DwarFS. Reject: master-server service architecture; LimniFS is a format, not
a cluster.

**Redox TFS**. Take: Rust modularity ambition. Reject: bundling too many
novel ideas so none finished — hence the walking-skeleton phasing (Phase 0
ships a boring reader first) and the spec/conformance split.

**ZFS/btrfs** (contrast class). Take: checksumming everything, snapshots as
first-class derivations (LimniFS deltas are the immutable-image analog).
Reject for our purposes: block-device coupling, mutable-state complexity —
LimniFS deliberately has no volume layer.

## 3. The five differentiators (why LimniFS exists at all)

1. **Identity/representation partition** (§4 architecture): no extant image
   format separates "what the bytes are" from "how they are stored" this
   strictly. It is what makes every other differentiator composable.
2. **Staged compression**: DwarFS-class ratios with tar-class write latency.
   Nobody else re-compresses in the background while keeping identity stable.
3. **Metadata-only flatten**: folding a delta chain costs O(metadata) with
   zero data movement. OCI, git, and DwarFS all move bytes to merge.
4. **Crypto/redundancy/DMS in the format**: AEAD registry, per-recipient HPKE,
   per-slab RS, dead man's switch — as registry-gated representations, not
   bolt-ons. borg/restic prove demand; no mountable image format has it.
5. **Locator-agnostic distribution**: the same image mounts from a file, an
   HTTP static host, S3, or IPFS with mirror racing — streaming-native, no
   full download, lying mirrors detected by construction.

## 4. Honest weaknesses (what others do better)

- **Kernel-native random read**: EROFS beats any FUSE path; our composefs
  path closes most of the gap on Linux only.
- **Ecosystem/tooling**: SquashFS and OCI are everywhere; LimniFS starts with
  one CLI and one consumer (tebako).
- **Mutable workloads**: anyone needing read-write needs ZFS/btrfs/ext4,
  full stop. LimniFS will never serve them — that is positioning, not a bug.
- **Maturity of threat model**: borg/restic have years of adversarial review;
  LimniFS's crypto is new and must earn trust via the conformance/fuzz
  program and external audit before Phase 2 ships encrypted images broadly.
