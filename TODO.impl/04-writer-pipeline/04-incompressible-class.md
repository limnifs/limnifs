---
Component: 04-writer-pipeline
Task: 04-incompressible-class
Status: done (2026-08-02, session 34)
Depends on: 04-classifier-seine
Unblocks: —
Source: docs/dwarfs-multicodec-investigation.md (Tier 1)
Fix landed: limnifs-write/src/classifier.rs — Class::Incompressible
  added; entropy ≥ 6.5 with no magic → STORE.
---

# 04-incompressible-class — Detect random/encrypted bytes, skip compression

## Problem

LimniFS's classifier routes content with entropy ≥ 7.5 (no magic
match) to the `Compressed` class → STORE. But random/encrypted data
with entropy in the **6.5–7.5** range lands in `Binary` → LZ4 →
wastes CPU attempting compression that won't help.

DwarFS has a dedicated `incompressible_categorizer` for exactly
this case. It routes to the `null` codec (store) and skips the
per-block compression attempt entirely.

## Approach

Add a new class `Incompressible` to the seine classifier:

```rust
pub enum Class {
    Text,
    Code,
    Binary,
    Compressed,
    Media,
    Sparse,
    Incompressible,  // NEW
}
```

Detection rule: entropy ≥ 6.5 AND no magic match AND not detected
as Compressed by magic. (Compressed stays at ≥ 7.5 to leave room
for actually-compressed data with slightly lower entropy.)

Route `Incompressible` → CODEC_STORE in `process_file`.

## Implementation sketch

1. Add `Class::Incompressible` to the enum.
2. Update `classify()`:
   ```rust
   if entropy >= INCOMPRESSIBLE_THRESHOLD {
       if entropy >= HIGH_ENTROPY_THRESHOLD {
           return Class::Compressed;  // likely real compressed data
       }
       if no_magic_match {
           return Class::Incompressible;  // random/encrypted
       }
   }
   ```
3. Update `process_file`'s codec routing:
   ```rust
   _ => CODEC_STORE,  // catches Incompressible + Compressed + Media
   ```
4. Update the class id encoding (`to_id()`) and string form.

## Acceptance criteria

- Synthetic `random` dataset (100 MB of xorshift output) routes to
  STORE without attempting LZ4 first. Bench create time on `random`
  drops by ≥ 20%.
- Real-world encrypted files (`gpg --encrypt` output, OpenSSL
  blobs) route to STORE.
- Real compressed files (gzip, zstd, xz streams) still route to
  `Compressed` (not `Incompressible`).
- Existing tests pass.

## CI evidence required

`benchmark.yml` quick mode shows `random` create time drops.
