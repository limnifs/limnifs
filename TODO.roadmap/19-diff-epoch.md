# diff epoch

- **Priority:** P2
- **Depends on:** 02
- **Estimated effort:** see detail

## Goal

Epoch diff command.

## Detail

limni diff-epoch <image> <epoch-a> <epoch-b>. Shows what operations changed between two epochs. Uses roaring bitmaps for O(1) per-inode diff queries.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
