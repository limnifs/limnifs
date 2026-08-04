# 13 — omnizip: PPMd context tree init

- **Priority:** P2
- **Side:** omnizip (file as proposal)
- **Est. effort:** 3d (omnizip-side)

## Problem

omnizip-ppmd's `PpmModel::with_memory_budget(order, max_bytes)`
allocates a context tree of up to `max_bytes` nodes. For a 256 MB
budget (max-ratio profile), this is a large allocation with O(N)
initialisation time. On 1 MB text fixtures, the init time is 100ms+
— negligible on large inputs but significant on many small chunks.

## Proposal

1. Lazy node allocation: allocate the context tree in pages (4 KB
   each). Only touch a page when a context actually maps to it.
2. Model reset for small inputs: if the input is < 64 KB, use a
   smaller model (16 MB budget) regardless of config. Adaptation
   hasn't converged; the larger tree just wastes init time.

## Expected impact

- Small-chunk PPMd encode: 2–3× faster init.
- Ratio on small chunks: unchanged or slightly better (smaller
  model adapts faster).

## Acceptance

- [ ] PPMd init time on 4 KB chunks: < 1ms.
- [ ] Ratio on 1 MB text: unchanged.
