# 04 — BCJ composite codecs (filter + codec pipelines)

- **Status:** pending
- **Phase:** 1
- **Depends on:** 04-omnizip-new-algos finding 3, 04-bcj-categorizer-routing
- **Design refs:** §6, 2026-throughput-roadmap.md §3, omnizip-filters
- **Priority:** P0

## Goal

`omnizip-filters` ships seven BCJ (Branch / Call / Jump) filters
implementations that convert relative call/branch addresses in
executable code to absolute values. Applied before LZ4/ZSTD/LZMA,
they typically improve ratio on ELF/PE/Mach-O binaries by 20–40%.

Today LimniFS wires only the shuffle filters (via `shuffle_lz4`,
`bitshuffle_lz4`, `shuffle_zstd` composites). The BCJ filters are
not wired. This TODO lands the composite codec layer; routing is
`04-bcj-categorizer-routing.md`.

## Design

### Codec id allocation

The wire format's `codec` byte is u8. Current allocation: 0x00..=0x12.
Reserve 0x20..=0x2F for BCJ composites.

| Id | Codec | Pipeline |
|---|---|---|
| 0x20 | `BCJ_X86_LZ4` | bcj_x86 → lz4 |
| 0x21 | `BCJ_X86_ZSTD` | bcj_x86 → zstd |
| 0x22 | `BCJ_X86_LZMA` | bcj_x86 → lzma |
| 0x23 | `BCJ_ARM64_LZ4` | bcj_arm64 → lz4 |
| 0x24 | `BCJ_ARM64_ZSTD` | bcj_arm64 → zstd |
| 0x25 | `BCJ_ARM_LZ4` | bcj_arm → lz4 |
| 0x26 | `BCJ_ARM_ZSTD` | bcj_arm → zstd |
| 0x27 | `BCJ_PPC_LZ4` | bcj_powerpc → lz4 |
| 0x28 | `BCJ_SPARC_LZ4` | bcj_sparc → lz4 |
| 0x29 | `BCJ_IA64_LZ4` | bcj_ia64 → lz4 |
| 0x2A | `BCJ_ARM_THUMB_LZ4` | bcj_arm_thumb → lz4 |

(Bcj2 splits into 5 streams and doesn't fit the single-stream
codec shape. Defer until a multi-stream codec format exists.)

### Composite codec shape

Follow the existing `ShuffleLz4Codec` pattern
(`limnifs-core/src/codec/shuffle_lz4.rs`):

```rust
pub struct BcjX86Lz4Codec;

impl Codec for BcjX86Lz4Codec {
    fn id(&self) -> u8 { CODEC_BCJ_X86_LZ4 }
    fn name(&self) -> &'static str { "bcj-x86+lz4" }
    fn min_compress_size(&self) -> usize { 1024 }

    fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
        let filtered = omnizip_filters::BcjX86Filter.encode(plaintext);
        let inner = crate::codec::compress(CODEC_LZ4, &filtered)?;
        // Prefix with filtered length so the LZ4 decoder validates.
        let mut out = Vec::with_capacity(4 + inner.len());
        out.extend_from_slice(&(filtered.len() as u32).to_le_bytes());
        out.extend_from_slice(&inner);
        Ok(out)
    }

    fn decompress(&self, compressed: &[u8], _expected_len: u32) -> Result<Vec<u8>, CoreError> {
        // Read prefix, LZ4-decompress, BCJ-decode.
    }
}
```

### Trait extraction (DRY)

The shuffle and BCJ composites share the same shape:
filter.encode → codec.compress → length-prefix. Extract a
`FilterCodecComposite<F: Filter, C: Codec>` generic to avoid 11
near-identical impls. This is a follow-up TODO
(`04-filter-codec-composite-trait.md`, P1) once two composites
exist.

## Notes

- The BCJ filters' `encode`/`decode` are exact inverses — round-trip
  is provable by construction.
- The composite's compressed output starts with a 4-byte LE length
  prefix (filtered-bytes count) so the LZ4 decoder validates output
  length even though `_expected_len` (the original plaintext length)
  isn't the LZ4 output length.
- Filters are deterministic (omnizip-filters's `Filter` trait
  guarantees), so `DropId = BLAKE3(plaintext)` is stable.

## Acceptance

- [ ] `CODEC_BCJ_X86_LZ4 = 0x20` constant exists in
      `limnifs-core/src/codec/mod.rs`.
- [ ] `BcjX86Lz4Codec` (and at least one ARM64 variant) registered
      in `CodecRegistry::default`.
- [ ] Round-trip test passes on a synthetic executable-like fixture
      (lots of relative call addresses).
- [ ] BCJ+LZ4 beats plain LZ4 by ≥ 20% ratio on the same fixture.
- [ ] A second test on a real Linux kernel `vmlinux` (subset)
      confirms the win is not fixture-specific.

## Why LimniFS cares

`vmlinux`, Docker images of OS distros, language runtimes (Python,
Node, Ruby) — all are heavy on executable code that doesn't
compress well with general-purpose codecs. BCJ is the single
highest-ratio win available for the `binary` content class.

## Effort estimate

- 1 day for 3-4 composites (X86 + ARM64 × LZ4 + ZSTD).
- 1 day for tests + benchmark.
- 1-2 days follow-up for the trait extraction (separate TODO).

## Related

- `04-bcj-categorizer-routing.md` — wires the categorizer to pick
  the right BCJ composite automatically.
- omnizip-rs proposal: none needed; the filters are already
  published.
