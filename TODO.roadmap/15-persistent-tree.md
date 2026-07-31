# persistent tree

- **Priority:** P2
- **Depends on:** 02
- **Estimated effort:** see detail

## Goal

Persistent HAMT for epoch trees.

## Detail

Use Hash Array Mapped Trie for the filesystem tree. Each epoch shares structure with its parent. Memory = O(changes), not O(total files). O(log32 N) lookup at any epoch.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green
