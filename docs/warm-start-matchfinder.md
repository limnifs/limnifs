# Warm-start matchfinder: investigation outcome (TODO.features/09)

**Status: blocked-upstream. The dictionary remains the only
cross-chunk redundancy mechanism LimniFS can express today.**

Question: can per-chunk compression carry matchfinder state across
chunk boundaries, so chunk N's encoder starts with the hash/chain
state chunk N−1 built (instead of from zero)? This is the classic
"warm start" / sliding-solid-window trick; on files with
cross-chunk-local redundancy it buys ratio without giving up
per-chunk decode.

## What omnizip has (0.21.55)

The *finder-level* primitive exists and is exactly right:

- `omnizip-codecs` `HashChainMatchFinder::prime_until(pos)`
  (matchfinder.rs, added 0.21.55): inserts positions `[cur, pos)`
  into the hash table and chain WITHOUT searching — "replays the
  store side of `advance` so a per-chunk finder can reproduce the
  state a sequentially shared finder would hold at `pos`". The doc
  also proves the safe-truncation property (chains may end at
  SENTINEL where the sequential chain would reach a too-far node —
  identical search results).
- `omnizip-zstd` has the analogous `MatchState::seed_prefix(buf,
  prefix_len)` (encoder/match_finder.rs): seeds absolute positions
  for a prefix, sets `next_to_update = prefix_len`, so block
  compressors on later slices find prefix positions as candidates.
  This is what powers `compress_with_dict`.

## Why LimniFS cannot use it

1. **`seed_prefix` is `pub(crate)`.** No public omnizip-zstd API
   accepts prior-window bytes. The exported surface is
   `compress`, `compress_mt`, `compress_with_dict`, and
   `ZstdCompressor::compress` — all stateless per frame.
2. **`ZstdCompressor`'s state reuse is an allocation cache, not a
   semantic warm start.** `encode_frame_into` begins with
   `match_state.clear()` (block.rs; the comment says the caller
   "can reuse it across calls", meaning the table allocation —
   output is byte-identical to the stateless path).
3. **Same shape for the LZ family**: `prime_until` exists on the
   omnizip-codecs finder, but no compression entry point takes a
   primed finder as an argument.

## The format-side constraint (independent of upstream)

Even with an API, warm-started chunks are not independently
decodable: chunk N's bytes would depend on chunk N−1's content,
breaking the drop model (content-addressed, random-access,
dedupable by `DropId`). Making that work needs format-level design
— chain drops à la solid archives, with the chain encoded in the
manifest and honored by readers, like `dictionary_section` +
`DropRecord::dict_id` already are. The dictionary IS that design,
restricted to a fixed seed blob rather than a rolling window.

## Recommendation

- File an upstream issue asking for a public warm-start API:
  "expose `seed_prefix`/prior-window context on the zstd and LZ
  compress entry points (e.g. `compress_with_prefix(input,
  prefix)`)". `prime_until`'s doc already argues the
  determinism/equivalence case.
- Until then, cross-chunk redundancy stays with dictionaries
  (trained, adopted from base layers, or crafted). Note the
  pay-for-itself gate: on omnizip ≥ 0.21.32 the fast-tier matcher
  is strong enough that dictionaries rarely clear it
  (`dict_ratio_measurement` reports 0.0% on log-like corpora), so
  the practical cross-chunk win today is dedup at chunk
  boundaries, which FastCDC already provides.

Revisit when either (a) the upstream API lands, or (b) a workload
profile shows cross-boundary redundancy that FastCDC dedup misses
(rare, sub-min-size repeats).
