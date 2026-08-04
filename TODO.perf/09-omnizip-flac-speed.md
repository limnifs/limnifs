# 09 — omnizip: FLAC LPC encoder speed

- **Priority:** P0
- **Side:** omnizip (file as proposal)
- **Est. effort:** 5d (omnizip-side)

## Problem

Benchmark `wav-synthetic` (24 MB WAV): LimniFS 1.506s create vs
DwarFS 0.146s. LimniFS achieves **0.02% ratio** (5× better than
DwarFS's 0.10%), but the encoder is **10× slower**.

omnizip-flac's LPC encoder does full autocorrelation + Levinson-Durbin
recursion per block. libFLAC uses incremental updates + early exit
heuristics.

## Proposal

File to omnizip-rs:
1. Incremental autocorrelation: update the autocorrelation sums
   per sample rather than recomputing from scratch each block.
2. Early-exit heuristic: if the LPC residual estimate from order N
   is already worse than the best FIXED subframe, skip higher
   orders.
3. Block-size adaptation: use 4096-sample blocks for high-frequency
   content (less LPC benefit) and 8192 for low-frequency (more
   redundancy).

## Expected impact

- LimniFS WAV create: 1.5s → ~0.3s (within 2× of DwarFS).
- Ratio unchanged (output byte-identical; just faster encoder
  decisions).

## Acceptance

- [ ] omnizip-flac encode speed improves ≥ 3×.
- [ ] Output ratio unchanged (same LPC order selection + Rice
      partition layout).
- [ ] LimniFS WAV benchmark shows speed improvement.
