# Proposal: Libdeflate — pure-Rust parity codec for DEFLATE benchmarking

**Filed by:** LimniFS
**omnizip-rs crate:** new crate `omnizip-libdeflate`
**Severity:** codec id 0x000B is reserved but no crate published

## Problem

`omnizip-codecs::CodecId::LIBDEFLATE = 0x000B` reserves a slot
for a libdeflate-compatible DEFLATE codec, but no
`omnizip-libdeflate` crate exists. LimniFS can't use this slot
even though we have a use for it (see below).

## What libdeflate is

Libdeflate (Eric Biggers, 2016+) is a faster DEFLATE
implementation than zlib:

| Codec | Encode (MB/s) | Decode (MB/s) | Ratio |
|---|---:|---:|---|
| zlib -6 | 50 | 250 | 100% (baseline) |
| libdeflate -6 | 200 | 600 | 100% |
| libdeflate -12 | 30 | 600 | 98% |

The decode speed win is the headline: ~2.4× faster than zlib on
the same input. The codec is DEFLATE-compatible — same wire
format, only the encoder/decoder implementation differs.

## Why LimniFS cares

LimniFS doesn't need libdeflate for new images (we use ZSTD +
Brotli + LZ4). But we receive plenty of legacy DEFLATE content:

- `.zip` archives (DEFLATE inside).
- `.jar` / `.war` files (Java's default).
- HTTP responses with `Content-Encoding: gzip`.
- `.git/objects` (zlib-compressed).

Today we decode these via `omnizip-deflate` (which wraps
`miniz_oxide`). `miniz_oxide` is correct but ~2× slower than
libdeflate. A libdeflate-compatible codec would halve extract
time on these workloads.

## What LimniFS proposes

### Scope: decode-only is fine

LimniFS's primary use is **decoding** legacy DEFLATE content. The
encode side is a nice-to-have but not blocking. Land decode-only
first; encode later if benchmarks justify.

### Pure-Rust implementation

The C libdeflate uses table-driven Huffman decode with hand-tuned
SIMD paths. The pure-Rust equivalent:

- Table-driven Huffman decode with the SIMD batching proposed in
  [simd-huffman-wide.md](./simd-huffman-wide.md).
- Sliding-window match copy with 32 KB and 64 KB variants.

No GPL code; the libdeflate algorithm is public domain, the wire
format is RFC 1951.

### Crate skeleton

```
omnizip-libdeflate/
├── Cargo.toml          # name=omnizip-libdeflate, deps=[omnizip-codecs]
├── src/
│   ├── lib.rs          # LibdeflateCodec impl Codec
│   ├── decoder.rs      # RFC 1951 + window
│   └── encoder.rs      # Phase 2
```

Wire format: identical to `omnizip-deflate` (RFC 1951). Codec id
0x000B distinguishes "we promise libdeflate-level decode speed"
from miniz_oxide.

### Acceptance

- [ ] `omnizip-libdeflate` crate exists with `LibdeflateCodec`.
- [ ] Decode throughput ≥ 1.8× `omnizip-deflate` on Calgary
      `paper1` and Enwik8.gz.
- [ ] Output byte-identical to `omnizip-deflate::decompress`.
- [ ] (Phase 2) Encode at level 6 produces DEFLATE-compatible
      output that `gunzip` accepts.

## Why LimniFS cares

LimniFS's `import` workflow (planned under `09-legacy-frozen2`)
extracts `.zip` and `.jar` files into RW images. Decode speed
directly bounds the import throughput.

## Effort estimate

- Decode: 5 days (table-driven Huffman + sliding window).
- Encode: 7 days (level 1 lazy + level 6 optimal).
- Total: 12 days for full codec.

Decode-only first is a meaningful win in itself.

## Related

- libdeflate upstream: https://github.com/ebiggers/libdeflate
- RFC 1951: DEFLATE wire format.
- `omnizip-rs TODO.complete/83-simd-huffman-decode.md` — the SIMD
  batching proposal lands first.
