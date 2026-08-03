# 04 — LZ4 HC wiring (when omnizip ships a real impl)

- **Status:** pending (blocked on upstream)
- **Phase:** 2
- **Depends on:** omnizip-lz4 release with real HC encoder
- **Design refs:** 04-omnizip-new-algos.md finding 2
- **Priority:** P2

## Goal

`omnizip-lz4 0.13.1` ships `Lz4HcCodec` whose `compress` body is
identical to `Lz4FastCodec::compress` (both call
`lz4_flex::compress_prepend_size`). The HC match finder is not
invoked; ratio is identical to fast LZ4. Once omnizip ships a
real HC encoder, wire it as codec id `0x13`.

## Blocker

File an issue at https://github.com/omnizip/omnizip-rs asking for
the HC match finder to be exposed. The `lz4-flex` crate already
has `compress_hc_prepend_size`; omnizip just needs to call it.

## Acceptance

- [ ] Upstream omnizip-lz4 issue filed.
- [ ] When omnizip-lz4 ships a real HC, this TODO is unblocked and
      codec id `0x13 = CODEC_LZ4_HC` is registered.
