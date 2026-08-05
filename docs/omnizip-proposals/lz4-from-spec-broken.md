# omnizip-lz4: 0.14.18 from-spec encoder breaks on ≥64-byte inputs

- **omnizip version affected:** 0.14.18 only
- **omnizip version fixed:** **0.14.19** (omnizip-rs PR #115)
- **LimniFS version affected:** 0.2.33 (pinned omnizip-lz4 to 0.14.17)
- **LimniFS version fixed:** 0.2.34 (pin removed)
- **Filed:** 2026-08-05
- **Status:** **RESOLVED upstream** — 2026-08-05

## Summary

`omnizip-lz4` 0.14.18 shipped a from-spec LZ4 block encoder that
replaced the `lz4_flex` wrapper (omnizip-rs TODO 132). The new
encoder produced output its own decoder rejected with
`"literal data extends past input"` for inputs with match/literal
lengths exactly at the code-nibble-15 boundary.

## Root cause

`write_length_ext(0)` wrote nothing, but the LZ4 spec requires at
least one extension byte when the code nibble equals 15. omnizip-rs
PR #115 fixed it to always write the final byte, plus added 5
regression tests covering boundary lengths, the 64-byte threshold,
and long matches.

## Resolution

omnizip-rs PR #115 / release 0.14.19. LimniFS 0.2.34 removes the
Cargo.lock pin and absorbs the from-spec encoder. The full LZ4
test suite (19 tests) passes on the unpinned 0.14.19.

## Historical reproduction (0.14.18)

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

## Historical side effects on LimniFS

Two `limnifs-core` tests failed on 0.14.18:

- `codec::tests::lz4_round_trips` — 143-byte Lorem Ipsum
- `codec::shuffle_lz4::tests::round_trips_float32_array` — composite codec using LZ4 internally

## Historical workaround (LimniFS 0.2.33)

Cargo.lock pinned `omnizip-lz4` to 0.14.17 (lz4_flex wrap, works
correctly). All other omnizip-* crates at 0.14.18.

## Acceptance criteria (all met)

1. ✅ The 143-byte Lorem Ipsum input round-trips through
   `Lz4FastCodec` without error.
2. ✅ All 19 LimniFS LZ4-using tests pass without the lockfile pin.
3. ✅ The `lz4_flex`-removal goal (TODO 132) is preserved — the
   from-spec encoder is the only LZ4 implementation in the dep tree.
