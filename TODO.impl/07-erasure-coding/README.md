# 07 — erasure-coding

`limnifs-ec`: slab-level Reed-Solomon redundancy (design §8). A representation
concern: EC changes how bytes are stored, never what they are.

- **Phase:** 3
- **Crate:** `limnifs-ec`
- **Design refs:** §8 (EC), §4 (identity rule), §10.2 (IPFS churn survival)

## Responsibilities (MECE)

**Owns:**

- Per-slab Reed-Solomon (k+m) encode/decode behind a `Ec` trait (OCP: future codes — e.g. fountain/LT — register alongside).
- Shard layout within/ across slab objects as recorded in the manifest (schema = 01).
- Repair: reconstruct a slab from any k shards; re-emit missing shards to locators (with 08).

**Does NOT own:** when EC is applied (tier policy, 04), where shards live (locator manifests, 01/08), drop identity (BLAKE3 of plaintext regardless).

## Public surface

```rust
trait Ec { fn id(&self) -> EcId; fn encode(&self, slab: &[u8], k: u8, m: u8) -> Vec<Shard>; fn reconstruct(&self, shards: &[Option<Shard>]) -> Result<Vec<u8>>; }
```

## Invariants

- EC parameters (k, m) are per-slab manifest metadata; mixed-EC images are legal.
- Reconstruction verifies against the slab's recorded hash before yielding bytes — EC is availability, the hash is integrity.

## Tasks

- [07-reed-solomon-slabs.md](07-reed-solomon-slabs.md)
- [07-ec-repair.md](07-ec-repair.md)
