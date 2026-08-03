# 04 — Filter-codec composite trait (DRY extraction)

- **Status:** pending
- **Phase:** 2
- **Depends on:** 04-bcj-composite-codecs
- **Design refs:** 2026-throughput-roadmap.md §9
- **Priority:** P1

## Goal

The shuffle composites (`ShuffleLz4Codec`, `BitshuffleLz4Codec`,
`ShuffleZstdCodec`) and the proposed BCJ composites
(`BcjX86Lz4Codec`, `BcjX86ZstdCodec`, …) share an identical shape:

```text
compress:   filter.encode(plaintext) → codec.compress → length-prefix
decompress: read length → codec.decompress → filter.decode → plaintext
```

Today each composite duplicates ~50 lines. After
`04-bcj-composite-codecs` lands, there will be 11 such impls.
Extract a generic to collapse them.

## Design

```rust
pub trait FilterCodecComposite: Codec {
    type Filter: omnizip_filters::Filter;
    type InnerCodec: Codec;

    fn filter() -> Self::Filter;
    fn inner_codec() -> Self::InnerCodec;
}

pub fn compress<F: FilterCodecComposite>(
    plaintext: &[u8],
) -> Result<Vec<u8>, CoreError> {
    let filtered = F::filter().encode(plaintext);
    let inner = F::InnerCodec::compress(&F::inner_codec(), &filtered)?;
    let mut out = Vec::with_capacity(4 + inner.len());
    out.extend_from_slice(&(filtered.len() as u32).to_le_bytes());
    out.extend_from_slice(&inner);
    Ok(out)
}

pub fn decompress<F: FilterCodecComposite>(
    compressed: &[u8],
) -> Result<Vec<u8>, CoreError> {
    // read prefix, inner.decompress, filter.decode
}
```

Each composite is then a 10-line specialisation:

```rust
pub struct BcjX86Lz4Codec;
impl Codec for BcjX86Lz4Codec {
    fn id(&self) -> u8 { CODEC_BCJ_X86_LZ4 }
    fn name(&self) -> &'static str { "bcj-x86+lz4" }
    fn compress(&self, p: &[u8]) -> Result<Vec<u8>, CoreError> { self::compress::<Self>(p) }
    fn decompress(&self, c: &[u8], _e: u32) -> Result<Vec<u8>, CoreError> { self::decompress::<Self>(c) }
}
impl FilterCodecComposite for BcjX86Lz4Codec {
    type Filter = omnizip_filters::BcjX86Filter;
    type InnerCodec = crate::codec::lz4::Lz4Codec;
    fn filter() -> Self::Filter { omnizip_filters::BcjX86Filter }
    fn inner_codec() -> Self::InnerCodec { crate::codec::lz4::Lz4Codec }
}
```

## Notes

- The trait's `filter()` and `inner_codec()` constructors return
  by value to avoid `&self` lifetime gymnastics. The types are
  zero-sized or near-zero-sized.
- This is a DRY refactor — no behaviour change. All existing
  composite tests must continue to pass byte-identically.

## Acceptance

- [ ] `FilterCodecComposite` trait exists in
      `limnifs-core::codec::composite`.
- [ ] Generic `compress`/`decompress` functions exist alongside.
- [ ] At least 3 existing composites (ShuffleLz4, ShuffleZstd,
      BitshuffleLz4) refactored to use the trait.
- [ ] The 11 BCJ composites from `04-bcj-composite-codecs` use
      the trait.
- [ ] All round-trip tests pass; output byte-identical to pre-refactor.

## Why LimniFS cares

- 14 composites × 50 lines = 700 lines eliminated.
- Adding a new composite (e.g. `DeltaLz4`) becomes 10 lines.
- The trait makes it obvious which composites exist (search for
  `impl FilterCodecComposite for`).

## Effort estimate

1 day:
- 4h: trait + generic functions + tests.
- 4h: migrate 14 composites.

## Related

- `04-bcj-composite-codecs.md` — the work that creates the
  duplication this refactor removes.
- `limnifs-core/src/codec/shuffle_lz4.rs` — current pattern.
