# omnizip-lz4: 0.14.18 from-spec encoder breaks on ≥64-byte inputs

- **omnizip version affected:** 0.14.18 only
- **omnizip version fixed:** pending (filed upstream 2026-08-05)
- **LimniFS version:** 0.2.33 (workaround: pin omnizip-lz4 to 0.14.17)
- **Filed:** 2026-08-05
- **Status:** Open — awaiting upstream fix

## Summary

`omnizip-lz4` 0.14.18 ships a from-spec LZ4 block encoder that
replaces the `lz4_flex` wrapper (omnizip-rs TODO 132). The new
encoder produces output that its own decoder rejects with
`"literal data extends past input"` for inputs above ~64 bytes
with certain content patterns.

## Reproduction

```rust
let codec = omnizip_lz4::Lz4FastCodec;
let data = b"Lorem ipsum dolor sit amet, consectetur adipiscing elit. \
             Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.";
// 143 bytes
let compressed = omnizip_codecs::Codec::compress(
    &codec, data, omnizip_codecs::CompressionLevel::default(),
).expect("compress");
omnizip_codecs::Codec::decompress(&codec, &compressed, data.len() as u32)
    .expect_err("Adler32Mismatch"); // errors here
```

Error: `codec 0x0001: decode failed — lz4 block decode failed: literal data extends past input`.

Shorter inputs (~61 bytes) round-trip correctly. The failure mode
suggests an issue with the encoder's match-or-literal decision at
certain block-boundary / pattern combinations.

## Side effects on LimniFS

Two `limnifs-core` tests fail:

- `codec::tests::lz4_round_trips` — 143-byte Lorem Ipsum
- `codec::shuffle_lz4::tests::round_trips_float32_array` — composite codec using LZ4 internally

## Workaround in LimniFS

Pin `omnizip-lz4` to 0.14.17 in `Cargo.lock`. The 0.14.17 release
uses the `lz4_flex` wrap (works correctly). All other omnizip-*
crates remain at 0.14.18 (LZMA ResetMode, Snappy encoder, etc. —
none affected by this bug).

When upstream ships the fix, unpin and remove the lockfile override.

## Acceptance criteria (upstream)

The fix is shipped when, on `omnizip-lz4` ≥ next patch:

1. The 143-byte Lorem Ipsum input round-trips through
   `Lz4FastCodec` without error.
2. All 19 LimniFS LZ4-using tests pass without the lockfile pin.
3. The `lz4_flex`-removal goal (TODO 132) is preserved — the
   from-spec encoder is the only LZ4 implementation in the dep tree.

## Why this matters

LZ4 is the most-used codec in LimniFS profiles (in 7 of 9 profiles
as the binary chunk codec or the `skip_chunking` fast path). A
broken LZ4 encoder breaks archive correctness for any image
produced with the affected version.
