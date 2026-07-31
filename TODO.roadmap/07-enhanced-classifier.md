# 07 — Enhanced seine classifier

- **Priority:** P1
- **Depends on:** —
- **Estimated effort:** 1 day

## Goal

Extend the seine classifier to detect more content types and route them
to the best codec automatically.

## New classes

| Class | Detection method | Best codec |
|---|---|---|
| Audio (PCM/WAV) | RIFF magic + PCM format | store (already compressed formats) or FLAC (if available) |
| Raw image (TIFF/RAW) | magic bytes | store (already has pixel data; specialized codecs future) |
| Structured (JSON/YAML/TOML) | syntax probing | zstd (repetitive structure compresses well) |
| Database (SQLite/LevelDB) | magic bytes | xz (structured binary) |

The classifier returns a class; the codec map (task 06) determines the
codec per class. Default codec map routes new classes appropriately.

## Acceptance

- Classifier detects at least 8 content classes (currently 6)
- Per-class codec selection demonstrably better than uniform codec
- Backward compatible: existing 6 classes unchanged
