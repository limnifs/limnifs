# 04 — LZ4 HC wiring

- **Status:** DONE (2026-08-04, omnizip 0.14.1 published)
- **Phase:** 2
- **Depends on:** omnizip-lz4 release with real HC encoder
- **Design refs:** 04-omnizip-new-algos.md finding 2, docs/omnizip-proposals/lz4-hc.md
- **Priority:** ~~P2~~ closed

## Resolution

omnizip-rs accepted LimniFS's LZ4 HC proposal #1 and shipped a real
hash-chain HC encoder in **omnizip-lz4 0.14.1**:

- File: `omnizip-rs/omnizip-lz4/src/hc.rs` (~200 LOC).
- Algorithm: 16-bit hash table + parallel hash-chain + greedy match
  selection with lazy look-ahead. `MAX_CHAIN_LENGTH = 256` per
  position. Pure Rust, byte-compatible with the fast LZ4 decoder.
- Deterministic, no unsafe code, no GPL.

LimniFS action (landed in PR #141):

1. Bumped `omnizip-lz4 = "0.14.1"` in `limnifs-core/Cargo.toml`.
2. Assigned codec id `0x13 = CODEC_LZ4_HC` in
   `limnifs-core/src/codec/mod.rs`.
3. Added `limnifs-core/src/codec/lz4.rs::Lz4HcCodec` wrapping
   `omnizip_lz4::Lz4HcCodec`.
4. Registered in `CodecRegistry::default`.
5. Three tests:
   - `hc_beats_fast_on_non_rle_friendly_input` — HC strictly beats fast.
   - `hc_decodes_through_fast_decoder` — wire format compatible.
   - `hc_round_trips` — end-to-end.

What's NOT done in this PR (filed as future work):
- Adding LZ4 HC to profile tournament lists (`max-ratio` text path).
- `process_file` routing binary chunks through HC when categorizer
  hasn't claimed them. Both are profile config changes, not code.

## Original problem (preserved for context)

`omnizip-lz4 0.13.1` shipped `Lz4HcCodec` whose `compress` body was
identical to `Lz4FastCodec::compress` — both called
`lz4_flex::compress_prepend_size`. The HC match finder was never
invoked; ratio was identical to fast LZ4. LimniFS verified this
directly: 50 KB mixed input → fast=461 bytes, hc=461 bytes
(identical). The proposed fix was a one-line change to call
`compress_hc_prepend_size`, but omnizip-rs went further and
implemented a real hash-chain HC encoder from scratch.

## Acceptance

- [x] Upstream omnizip-rs issue filed (proposal #1, accepted).
- [x] Real HC encoder committed locally in omnizip-rs (`2d883db`).
- [x] Upstream published `omnizip-lz4 0.14.1` with the encoder.
- [x] LimniFS bumped the dep and wired codec id `0x13`.

## Related

- Proposal: `docs/omnizip-proposals/lz4-hc.md`
- Upstream commit: `omnizip-rs@2d883db`
- Upstream release: `omnizip-rs@0.14.1`
