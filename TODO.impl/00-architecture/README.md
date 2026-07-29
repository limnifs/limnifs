# 00 — architecture (SSOT for interfaces)

Normative architecture for LimniFS. Where a component README and these
documents disagree, **these documents win** — fix the README, not this.

- **Phase:** 0 (kept current every phase)
- **Owns:** module interaction points, interface contracts, algorithm specifications, positioning vs. extant filesystems
- **Does NOT own:** implementation, work state (root README), wire format (01-spec — but these docs reference it)

## Documents

| File | Content |
|---|---|
| [00-overview.md](00-overview.md) | What LimniFS is (an image-format VFS, not a block-device FS), the layer cake, module map, data-flow overview |
| [01-interfaces.md](01-interfaces.md) | Every interaction point between modules: trait contracts, call sequences (read/write/merge/mount/verify paths), error model, observability |
| [02-algorithms.md](02-algorithms.md) | Normative algorithm specs: FastCDC, BLAKE3 identity, Merkle construction, classification, AEAD/nonce/AD, HPKE, Reed-Solomon layout, delta diff, flatten, turnover, GC, DMS primitives — with complexity bounds and pseudocode |
| [03-comparison.md](03-comparison.md) | How LimniFS differs from extant filesystems and image formats: DwarFS, SquashFS, EROFS, composefs, OCI, casync, borg/restic, git, IPFS/UnixFS, Taobao TFS, Redox TFS, block-device filesystems |

## Rules

1. These documents are *prescriptive*: interfaces are specified here before they are implemented (spec-first applies to APIs, not just wire format).
2. Changing an interface = changing these docs first, in the same PR as the code or before it.
3. Diagrams are ASCII so they render everywhere (terminal, GitHub, printed docs) and diff cleanly.
4. Algorithm specs in `02-algorithms.md` are the acceptance reference for conformance vectors in `02-conformance`: vectors are generated to exercise every stated boundary condition.
