# Proposal: LZMA — optimal parser for max-ratio parity with xz CLI

**Filed by:** LimniFS
**omnizip-rs crate:** `omnizip-lzma`
**Severity:** feature gap (current encoder uses lazy parsing; ~5-10% behind xz CLI on text)

## Problem

`omnizip-lzma 0.13.1` ships a real LZMA encoder using lazy parsing
(match-then-rep0 tracking). This works (LimniFS uses it for
`CODEC_XZ` and gets 52% on text fixtures), but the `xz` CLI's
optimal parser (NICE_LEN=273, optimal parsing with backward
references) produces 5–10% smaller output on text and source code.

`WriteConfig::codec_tunables.lzma.use_optimal_parser` is wired
through LimniFS's `CodecTunables::lzma_dict_mb` field today, but
the omnizip-lzma encoder ignores it — there's no optimal-parser
code path to call.

## Proposed design

LZMA's optimal parser is a dynamic-programming algorithm: for each
byte position, compute the cheapest encoding (literal vs. match of
length N at distance D) using a cost model derived from the
probability state. Sources:

- Igor Pavlov's `LZMA SDK` (public-domain C++) — algorithm basis.
- Martelocc (2024) — modern Rust port for reference (not copied).

### Phased delivery

**Phase 1 — Cost model scaffold** (~2 days)

- Extract `LzmaProbState` from the current encoder.
- Add `literal_cost(pos, byte)` and `match_cost(pos, len, dist)` in
  terms of the prob state.
- No change to encode path yet.

**Phase 2 — Optimal parse** (~3 days)

- For each byte, compute the optimal parse via backward DP over a
  sliding window of `OPTS = 4096` positions.
- Track rep0/rep1/rep2 states through the parse.
- Emit the same LZMA token stream as lazy parsing — wire format
  is unchanged, only the parse decisions differ.

**Phase 3 — Wire level → parser** (~1 day)

```rust
pub fn lzma_compress_with_parser(
    plaintext: &[u8],
    level: LzmaLevel,
    parser: LzmaParser,  // Lazy vs Optimal
) -> Result<Vec<u8>, LzmaError>;
```

Map `level ≥ 6` to optimal parser; `level < 6` to lazy (faster).
Decode path is unchanged.

## Acceptance

- [ ] Calgary `book1` (768 KB): optimal parser ≥ 5% smaller than
      current lazy parser.
- [ ] Calgary `paper1` (53 KB): ≥ 3% smaller (smaller corpus,
      less benefit).
- [ ] Encode time within 3× of lazy (optimal parsing is slow).
- [ ] Decoder byte-identical (no wire-format change).

## Why LimniFS cares

LimniFS's `max-ratio` profile routes text through a tournament
that includes PPMd7, Brotli q11, and LZMA. LZMA losing 5–10% to
`xz` CLI means Brotli or PPMd wins on text — but PPMd is slow to
decode, and Brotli q11 is slow to encode. Optimal-parser LZMA
gives us the best text ratio with reasonable encode/decode speed.

## Effort estimate

6 days total (per phased delivery above).

## Related

- Igor Pavlov LZMA SDK: https://www.7-zip.org/sdk.html
- LZMA spec: `LZMA spec.txt` in the 7-Zip source distribution.
- Martelocc Rust port: https://github.com/martelocc/lzma-rs (study
  reference only; not copied).
