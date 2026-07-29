# 00 — Architecture overview

## 0. What LimniFS is — and is not

LimniFS is a **virtual filesystem in image form**. A `.limni` image is a *file*
(or a set of objects) that lives **on** a host filesystem, object store, or
medium — never directly on a block device. LimniFS has no volume layer, no
journaling, no free-space bitmap, no device driver. It assumes durable byte
storage underneath and adds everything a block FS cannot: content addressing,
staged compression, authenticated encryption, deltas, erasure coding, and
locator-agnostic distribution.

```
        block-device filesystems              image-format VFSes
        (ext4, ZFS, btrfs, APFS)              (SquashFS, DwarFS, LimniFS)
                  ▲                                    ▲
        owns the disk                      rides on top of any of these
        mutable, rw                        immutable, content-addressed
        local only                         distributable by construction
```

This positioning is the root design constraint: because the substrate is
"some byte store behind a locator", everything in LimniFS is expressed as
**self-describing objects with hashes**, and nothing requires POSIX semantics
from below.

## 1. The layer cake (on-disk format)

```
┌────────────────────────────────────────────────────────────┐
│ MANIFEST (small, signed, custom binary)                     │
│  versions · feature flags · slab index · crypto params      │
│  EC params · delta linkage (base_root) · DMS policy         │
│  MERKLE ROOT  ← this hash is the image's identity           │
├────────────────────────────────────────────────────────────┤
│ FILESYSTEM METADATA (custom binary; deterministic Merkle    │
│  B-tree for the directory)                                  │
│  inodes · dir tree · xattrs · slice→drop maps · class info  │
├────────────────────────────────────────────────────────────┤
│ DROP STORE (bulk bytes)                                     │
│  slabs (4–64 MiB objects)                                   │
│  ┌─slab─┐ ┌─slab─┐ ┌─slab─┐        ┌─ EC shards (opt) ─┐   │
│  │drops…│ │drops…│ │drops…│  …     │ k+m Reed-Solomon  │   │
│  └──────┘ └──────┘ └──────┘        └───────────────────┘   │
└────────────────────────────────────────────────────────────┘
   may be detached: manifest over here, slabs behind locators
   (file / http / s3 / ipfs / p2p) over there
```

Two-level addressing: `slice → drop → (slab, offset, len, representation)`.

## 2. The module map (code)

Repo boundaries (org `limnifs`): everything below lives in the
`limnifs/limnifs` workspace **except** 01+02 (`limnifs/spec`), the Python
reference reader (`limnifs/limnifs-py`), and 09 (`limnifs/limnifs-frozen2`).
Repo separation exists only where it is load-bearing: independent spec
versioning, spec-only reader independence, and the license-scan boundary.

```
                        ┌───────────────┐
                        │   01 spec     │  schema + registries (SSOT)
                        │ (codegen)     │
                        └──────┬────────┘
                               ▼
                        ┌───────────────┐      traits      ┌──────────────┐
                        │ 03 core-reader│◄────────────────►│ 09 frozen2   │
                        │  manifest     │  (same Image/    │ (legacy      │
                        │  drop store   │   DropSource)    │  read-only)  │
                        │  overlay      │                  └──────────────┘
                        └──┬───┬───┬────┘
              consumes     │   │   │     consumes (as traits, never impls)
        ┌──────────────────┘   │   └──────────────────┐
        ▼                      ▼                      ▼
┌───────────────┐     ┌───────────────┐      ┌───────────────┐
│ 05 crypto     │     │ 08 locators   │      │ 07 erasure    │
│ AEAD registry │     │ file http s3  │      │ Reed-Solomon  │
│ HPKE sig DMS  │     │ ipfs · CAR    │      │ encode repair │
└──────┬────────┘     └──────┬────────┘      └──────┬────────┘
       │                     │                      │
       └──────────┬──────────┴──────────┬───────────┘
                  ▼                     ▼
        ┌───────────────┐     ┌───────────────┐
        │ 04 writer     │     │ 06 deltas     │
        │ chunk classify│     │ build flatten │
        │ ingest deepen │     │ turnover      │
        └──────┬────────┘     └──────┬────────┘
               └──────────┬──────────┘
                          ▼
        ┌─────────┬───────────────┬─────────────┐
        ▼         ▼               ▼             ▼
   ┌────────┐ ┌────────┐   ┌───────────┐ ┌────────────┐
   │ 10 cli │ │11 mount│   │ 12 tebako │ │13 ci       │
   │ limni  │ │ FUSE / │   │ press +   │ │ (orchestr. │
   │        │ │ compfs │   │ parity    │ │  only)     │
   └────────┘ └────────┘   └───────────┘ └────────────┘

   02 conformance ── black-box over everything, owned by none
```

Dependency rule (acyclic): arrows point downward only. `03` never imports
`04`; crypto/EC/locators are *consumed by* reader and writer through traits,
never linked upward. See root README invariant 4.

## 3. The pipeline (data flow, design §6)

```
 write path                          read path
 ──────────                          ─────────
 slices (files)                      request: path+range
      │                                   │
      ▼                                   ▼
 FastCDC chunking ──► drops       resolve: inode → slice → drops
      │                                   │
      ▼                                   ▼
 seine classify (class per drop)  per drop: slab index → (slab,off,len,rep)
      │                                   │
      ▼                                   ▼
 EPILIMNION: LZ4/store now         locator.fetch(range) ──► decode rep
      │  (write ≈ memcpy+hash)              │  (codec ⁻¹ ∘ AEAD open ∘ EC repair)
      │                                   ▼
      ▼ (background, policy-driven)  verify BLAKE3 == DropId
 METALIMNION → HYPOLIMNION                │
 re-encode per class, repack slabs        ▼
      │  identity NEVER changes      plaintext bytes
      ▼
 new Representation rows only
```

## 4. Identity and representations (design §4, restated as architecture)

Every module respects one partition:

- **Identity plane** — `DropId = BLAKE3(plaintext)`, `ManifestRoot` (Merkle),
  inode identities in deltas. Owned by 01 (schema) and 03 (verification).
- **Representation plane** — codec, AEAD, EC, locator placement. Owned by
  04/05/07/08 respectively. Anything in this plane may change without
  invalidating anything in the identity plane.

This partition is what makes deepening, encryption, EC, mirroring, and
delta-flatten mutually composable: each transforms representations while
holding identity constant.

## 5. Immutability and mutation

Images are immutable. All mutation is *derivation*:

- **delta** — derive a child manifest with tree ops + new drops (06)
- **flatten** — derive one manifest from a chain, references only (06)
- **turnover** — derive a standalone image, drops repacked (06 + 04)
- **deepen** — derive new representations, same identity (04)

Mounts are read-only (11). The CLI exposes only derivations (10). There is no
code path that mutates an image in place — that property is architectural,
not a policy flag.

## 6. Key architectural decisions (ADRs, condensed)

| # | Decision | Alternative rejected | Why |
|---|---|---|---|
| A1 | Identity = BLAKE3(plaintext) | hash of stored bytes | representations must be interchangeable; dedup across recipients/tiers |
| A2 | Slabs as storage unit, drops as identity unit | per-drop objects (IPFS-style) | per-object overhead and read amplification kill small files (Taobao TFS lesson) |
| A3 | Staged pipeline (LZ4 → deep codecs) | one-shot build-time compression (DwarFS) | quick write and quick read simultaneously; compression as re-runnable stage |
| A4 | Manifest detachable from drop store | single self-contained file only | cloud assembly, IPFS pinning, mirror racing become configuration |
| A5 | Registries as data for every variation point | enum + match in core | OCP: post-v1 additions without core modification |
| A6 | Three merge tiers incl. metadata-only flatten | full re-encode only | folding patches must cost O(metadata), seconds |
| A7 | Spec + conformance as separate artifacts | implementation-as-spec (DwarFS, Redox TFS) | monoliths can't evolve; multiple implementations keep the spec honest |
| A8 | no-std-adjacent minimal core, plugins outside | batteries-included reader | audit surface, embeddability (tebako static link) |
