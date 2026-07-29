# 03 — Drop-store reader

- **Status:** pending
- **Phase:** 0
- **Depends on:** 03-manifest-parser
- **Design refs:** §4 (identity), §5 (two-level addressing), §6 (read path)

## Goal

Slice → drop → (slab, offset, len, representation) resolution; representation
decoding via the codec registry (LZ4, zstd, store at Phase 0–1); BLAKE3 verify
against `DropId` on every read.

## Notes

- `DropSource` trait abstracts slab origin; file-backed impl here, remote via 08 later (encapsulation).
- Paged slab index: no allocation proportional to image size.
- Zero-copy decode into caller buffers where metadata records sizes.

## Acceptance

- Read amplification ≤ 1.25× on perf vectors (solid blocks excluded as recorded).
- Hash-mismatch injection vectors fail closed with `IntegrityError`, never yield bad bytes.
