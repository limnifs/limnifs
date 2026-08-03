# 04 — LZ4 HC wiring (when omnizip publishes the real encoder)

- **Status:** in_progress (encoder landed locally in omnizip-rs; pending crates.io publication)
- **Phase:** 2
- **Depends on:** omnizip-lz4 release with real HC encoder
- **Design refs:** 04-omnizip-new-algos.md finding 2, docs/omnizip-proposals/lz4-hc.md
- **Priority:** P2

## Update (2026-08-04)

The omnizip-rs maintainers accepted LimniFS's LZ4 HC proposal #1
and implemented a real HC encoder:

- Local commit: `2d883db fix(lz4): real HC encoder (LZ4 HC proposal #1)`
- File: `omnizip-rs/omnizip-lz4/src/hc.rs` (~200 LOC)
- Algorithm: hash-table 16-bit hash of every 4-byte window →
  parallel hash-chain → greedy match selection with lazy
  look-ahead → MAX_CHAIN_LENGTH = 256 per position. Pure Rust,
  byte-compatible with the fast decoder (lz4_flex::decompress_size_prepended).
- Deterministic, no unsafe code, no GPL.

## What's left

**Publication.** The local `Cargo.toml` still says `version = "0.13.1"`
(same as the published stub). When omnizip publishes `0.13.2` or
`0.14.0`, LimniFS picks up the real HC automatically by bumping
the workspace dep.

LimniFS action on publication:

1. Bump `omnizip-lz4 = "0.13.2"` (or whatever version) in
   `limnifs-core/Cargo.toml`.
2. Assign codec id `0x13 = CODEC_LZ4_HC` in
   `limnifs-core/src/codec/mod.rs`.
3. Add `limnifs-core/src/codec/lz4_hc.rs` wrapping
   `omnizip_lz4::Lz4HcCodec`.
4. Register in `CodecRegistry::default`.
5. Add LZ4 HC to the `max-ratio` tournament list (it should beat
   LZ4 fast on text at slower encode speed).
6. Add a behavioural test: HC output strictly smaller than fast
   on Calgary `paper1`.

## Original problem (preserved for context)

`omnizip-lz4 0.13.1` ships `Lz4HcCodec` whose `compress` body is
identical to `Lz4FastCodec::compress` — both call
`lz4_flex::compress_prepend_size`. The HC match finder is never
invoked; ratio is identical to fast LZ4. LimniFS verified this
directly: 50 KB mixed input → fast=461 bytes, hc=461 bytes
(identical). The proposed fix is a one-line change to call
`compress_hc_prepend_size`, but omnizip-rs went further and
implemented a real hash-chain HC encoder from scratch.

## Acceptance

- [x] Upstream omnizip-rs issue filed (proposal #1, accepted).
- [x] Real HC encoder committed locally in omnizip-rs.
- [ ] Upstream publishes a new version of omnizip-lz4 with the encoder.
- [ ] LimniFS bumps the dep and wires codec id `0x13`.

## Related

- Proposal: `docs/omnizip-proposals/lz4-hc.md`
- Upstream commit: `omnizip-rs@2d883db`
