# 04 — Chunker trait (algorithm-pluggable CDC)

- **Status:** pending
- **Phase:** 2
- **Depends on:** 04-chunking-fastcdc
- **Design refs:** §6, 2026-throughput-roadmap.md §1
- **Priority:** P1

## Goal

`FastCDC` is currently a concrete struct the writer instantiates
directly. 2026 CDC variants (Gear+SIMD, leap-based parallel CDC)
can't slot in without rewriting the writer. Promote chunking to a
trait so a future `GearChunker` or `SimdFastCDC` registers behind
the same interface.

## Design

```rust
pub trait Chunker: Send + Sync {
    fn chunk_slice(&self, data: &[u8]) -> Vec<Vec<u8>>;
    fn avg_chunk_size(&self) -> usize;
}

pub struct FastCDC { /* unchanged */ }
// impl Chunker for FastCDC { ... }
```

`WriteConfig::chunking` already carries size params; it would gain
a `name: String` field (default `"fastcdc"`) so a future
`"gear-simd"` selects the SIMD variant.

## Notes

- Profile the current FastCDC against a SIMD Gear implementation
  before adopting. The FastCDC paper's normalization already gives
  us most of the practical wins; SIMD might be a 2× CPU win on the
  hashing step alone.
- The trait is intentionally minimal — no streaming variant yet.
  Streaming lands with the pipeline-parallelism work
  (`04-pipeline-parallelism.md`).

## Acceptance

- [ ] `Chunker` trait exists in `limnifs-write::chunker`.
- [ ] `FastCDC` implements it.
- [ ] `WriteConfig::chunking` gains a `name` field defaulting to
      `"fastcdc"`.
- [ ] No behaviour change for existing profiles.
