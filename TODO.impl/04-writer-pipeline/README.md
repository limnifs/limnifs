# 04 — writer-pipeline

`limnifs-write`: the staged compression pipeline (design §6). Quick write now,
deep compression later, quick read always.

- **Phase:** 1
- **Crate:** `limnifs-write`
- **Design refs:** §6 (pipeline), §5 (slab packing), §16 (open questions 1–2)

## Responsibilities (MECE)

**Owns:**

- Content-defined chunking (FastCDC) of slices into drops; `DropId` computation (BLAKE3 of plaintext — identity rule).
- The *seine* classification pass: entropy/magic heuristics → class labels (text/code, binary, compressed, media, sparse) via a **classifier registry** (OCP: new classifiers register, pipeline unchanged).
- Epilimnion ingest: immediate LZ4/store write; write latency ≈ memcpy + hash.
- Deepening (metalimnion → hypolimnion): re-encode drops class-appropriately (zstd/lzma/brotli/store), emitting *new representations* — identity and references untouched.
- Slab packing and drop GC: two-level addressing maintained; unreferenced drops collected during turnover (with 06).

**Does NOT own:** delta semantics and merge operations (06), AEAD/key management (05, applied as a representation step), EC (07), manifest schema (01).

## Public surface

```rust
trait Classifier { fn classify(&self, head: &[u8], entropy: f32) -> ClassId; }
trait Sink { fn put_drop(&mut self, id: DropId, rep: Representation, bytes: &[u8]) -> Result<SlabRef>; }
struct Pipeline { classifiers: Registry<Classifier>, tiers: TierPolicy }
```

## Invariants

- A drop may gain representations but never loses its `DropId`; deepening is referentially transparent.
- Ingest never blocks on deepening; deepening is cancel-safe (partial work produces valid intermediate manifests).
- Classifier decisions are recorded per drop class, never baked into layout (reclassification is possible).

## Performance budget

- Ingest throughput ≥ 80% of memcpy+BLAKE3 baseline on warm cache.
- Deepening runs under an I/O budget (configurable IOPS/throughput cap) so reads stay fast.

## Tasks

- [04-chunking-fastcdc.md](04-chunking-fastcdc.md)
- [04-classifier-seine.md](04-classifier-seine.md)
- [04-ingest-epilimnion.md](04-ingest-epilimnion.md)
- [04-deepening-compactor.md](04-deepening-compactor.md)
- [04-slab-packing-gc.md](04-slab-packing-gc.md)
