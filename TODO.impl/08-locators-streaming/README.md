# 08 — locators-streaming

`limnifs-locator-*`: where bytes live and how they stream (design §10). The
core reader addresses slabs abstractly; locators resolve them.

- **Phase:** 2 (file/http/s3), 3 (ipfs/p2p)
- **Crates:** `limnifs-locator` (trait + file), `limnifs-locator-http`, `-s3`, `-ipfs`
- **Design refs:** §10.1 (registry), §10.2 (CAR/IPFS), §6 (read path), §1.1 (DwarFS read-amplification lesson)

## Responsibilities (MECE)

**Owns:**

- The `Locator` trait and scheme registry (OCP: new scheme = new crate, core untouched).
- `file:` (mmap), `http(s):` (range requests, read-ahead with slab index as seek map), `s3:`, `ipfs:`.
- Mirror racing: multiple locators per slab with priorities; hedged requests; failover mid-stream.
- CAR import/export (`limni export-car`/`import-car` backend) and multihash bridging of `DropId`s.
- Local content-addressed cache (feeds 11's composefs path).

**Does NOT own:** byte meaning (03), caching *policy* beyond transport heuristics, slab layout (01/04).

## Public surface

```rust
trait Locator { fn scheme(&self) -> Scheme; fn get(&self, r: &SlabRef, range: Range<u64>) -> impl Stream<Item = Result<Bytes>>; }
struct LocatorRegistry { /* scheme -> factory; mirrors raced per policy */ }
```

## Invariants

- Streaming-native: no full-slab download is ever required to serve a range read (solid-block extents from metadata are the only forced inflation).
- A lying locator is detected at the drop level (BLAKE3 verify) and demoted, never trusted silently (threat model, design §11).

## Performance budget

- Cold HTTP range read latency ≤ 2 RTTs after manifest open; read-ahead hides steady-state latency.

## Tasks

- [08-locator-trait-registry.md](08-locator-trait-registry.md)
- [08-http-range-streaming.md](08-http-range-streaming.md)
- [08-s3-locator.md](08-s3-locator.md)
- [08-ipfs-car.md](08-ipfs-car.md)
