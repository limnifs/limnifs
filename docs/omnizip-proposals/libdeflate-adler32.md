# omnizip-libdeflate: Adler-32 over compressed stream (RFC 1950 violation)

- **omnizip version:** 0.14.6 (current latest)
- **LimniFS version:** 0.2.26 (workaround in place)
- **Filed:** 2026-08-05
- **Status:** Open — awaiting upstream fix

## Summary

`omnizip_libdeflate::LibdeflateCodec::compress` produces a zlib
stream (RFC 1950) whose 4-byte Adler-32 trailer is computed over the
COMPRESSED deflate body instead of the original plaintext. RFC 1950
§9 specifies the Adler-32 is over the UNCOMPRESSED data.

miniz_oxide (used by `omnizip-deflate`) and `gzip -d` strictly
validate the Adler-32 trailer and reject streams produced by
`omnizip-libdeflate`.

`omnizip-libdeflate`'s own decoder happens to accept these streams
because its Phase 2 in-house inflate path strips the zlib wrapper
without validating the trailer. So self-roundtrip works, but
interop with any spec-strict decoder fails.

## Reproduction

```rust
let data = b"cross-decode test data ".repeat(20);
let libdeflate_compressed = omnizip_libdeflate::LibdeflateCodec::new()
    .compress(&data, CompressionLevel::default())?;
// miniz_oxide rejects this:
miniz_oxide::inflate::decompress_to_vec_zlib(&libdeflate_compressed)
    .expect_err("Adler32Mismatch");
```

Error: `DecompressError { status: Adler32Mismatch, ... }`.

## Root cause

`omnizip-libdeflate/src/lib.rs::wrap_zlib`:

```rust
fn wrap_zlib(deflate_stream: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(deflate_stream.len() + 6);
    out.push(0x78);
    out.push(0x9C);
    out.extend_from_slice(deflate_stream);
    let checksum = adler32(deflate_stream);  // BUG: should be plaintext
    out.extend_from_slice(&checksum.to_be_bytes());
    out
}
```

The function only has the compressed stream in scope; it would need
the plaintext passed in to compute the correct checksum.

## Workaround in LimniFS

`limnifs-core/src/codec/libdeflate.rs::LibdeflateCodec::compress`
re-computes the Adler-32 over the plaintext and patches the trailer
before returning. Output is byte-compatible with `gzip`/`zlib`/0x05.

When the upstream fix lands, drop the patch step from our wrapper.

## Acceptance criteria (upstream)

The fix is shipped when, on `omnizip-libdeflate` ≥ next patch:

1. `LibdeflateCodec::compress` output is accepted by
   `miniz_oxide::inflate::decompress_to_vec_zlib` without an
   `Adler32Mismatch` error.
2. `gzip -d` accepts the output without warning.
3. The LimniFS `libdeflate.rs` Adler-32 patch can be removed without
   `cross_decodes_with_deflate` failing.
