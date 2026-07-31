# 03 — Overlay resolver

- **Status:** done — limnifs-core::ContentHandle::SliceMap resolves via SlabView
- **Phase:** 0 (single-image), 2 (chains)
- **Depends on:** 03-drop-store-reader
- **Design refs:** §7 (tier-1 read-time overlay), §16 (open question 4)

## Goal

Walk a manifest chain (delta → base → …) applying tree ops to produce one
resolved `Tree`; enforce depth policy; reject cycles.

## Notes

- Rename semantics decided here per spec (01): first-class rename ops vs. remove+add compiled at build time — implement what spec says, nothing else (SSOT).
- O(depth) metadata per lookup; no data copying (tier 1 is read-time only).
- Cycle/depth violations are structured errors with the offending `ManifestRoot` in the message.

## Acceptance

- Chain vectors (depth 1–3) resolve identically in Rust and Python readers.
- Adversarial vectors (cycle, depth overflow, dangling `base_root`) fail with the specified errors.
