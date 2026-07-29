# 01 — Interfaces: every interaction point

Normative contracts between modules. Each interface lists: participants,
types exchanged, pre/post-conditions, error cases, and who may implement it
(OCP direction). Signatures are illustrative Rust; the binding semantics are
the text. Wire-format details are owned by `01-spec`; this document owns
*behavioral* contracts.

## 0. Shared semantic types (defined in 01-spec, used everywhere)

```rust
DropId        // BLAKE3(plaintext), multihash-compatible
SlabId        // per-image slab ordinal + content hash
ManifestRoot  // Merkle root over metadata + slab index
Tier          // Epilimnion | Metalimnion | Hypolimnion
Representation { codec: CodecId, aead: Option<AeadId>, ec: Option<EcId> }
SlabRef       // (SlabId, offset, len, Representation) + locator entries
Bytes<'a>     // zero-copy borrowed buffer
```

Rule: no module boundary crosses with a bare hash array or integer where a
semantic type exists.

## 1. Error model (all interfaces)

- One error enum per crate (`CoreError`, `WriteError`, `CryptoError`, …),
  `thiserror`-style, each variant carrying the semantic IDs involved
  (`DropId`, `SlabId`, `ManifestRoot`) — errors are *diagnosable*, not strings.
- Common cross-crate categories (mapped, never stringly):
  `UnsupportedFeature(flag)`, `Integrity { id }`, `Authentication { id }`,
  `Locator { scheme, cause }`, `Corrupt { offset, reason }`,
  `Policy { rule }`, `Unsupported(path)` for legacy adapter gaps.
- No panics on untrusted input anywhere below 10-cli; panics = bugs, hunted
  by the 02 fuzz corpus.
- Trait implementors may add variants; consumers must handle unknown variants
  via a `#[non_exhaustive]`-style catch-all (OCP on errors).

## 2. Interface inventory

| # | Interface | Provider | Consumers | Stability |
|---|---|---|---|---|
| I1 | `Image` / `Inode` / `Tree` | 03-core | 08, 10, 11, 12, 09 | frozen at v1 |
| I2 | `DropSource` | 03 (file), 08 (remote) | 03, 11 | frozen at v1 |
| I3 | `Codec` | 03 builtins + plugins | 03, 04 | registry-gated |
| I4 | `Aead` / `KeyWrap` / `Signer` / `Dms` | 05 | 03, 04, 10 | registry-gated |
| I5 | `Classifier` | 04 + plugins | 04 | registry-gated |
| I6 | `Sink` | 04 (file), 08 (locators) | 04, 06 | frozen at v1 |
| I7 | `DeltaBuilder` / `Flattener` / `Turnover` | 06 | 10, 12 | frozen at v1 |
| I8 | `Ec` | 07 | 03, 04, 08 | registry-gated |
| I9 | `Locator` / `LocatorRegistry` | 08 crates | 03, 04, 07 | registry-gated |
| I10 | legacy `Image` impl | 09 | 10, 11, 12 | internal |
| I11 | libtfs adapter shim | 12 | tebako | external contract |
| I12 | conformance harness protocol | 02 | 13 (CI) | frozen at v1 |
| I13 | codegen artifacts | 01 | all crates | generated |

"Registry-gated" = OCP extension point: new implementations register by ID;
interface itself never changes.

## 3. Contract details

### I1 — Image/Tree (03-core → consumers)

```rust
trait Image {
    fn root(&self) -> &Manifest;
    fn resolve(&self, path: &Path) -> Result<Inode, CoreError>;
    fn tree(&self) -> &Tree;                    // resolved view incl. overlays
}
trait Tree {
    fn iter(&self, dir: &Inode) -> Result<impl Iterator<Item = DirEntry>>;
    fn slice_map(&self, inode: &Inode) -> Result<SliceMap>;  // slice → [DropId]
}
```

- Pre: manifest validated at open (feature flags supported, structure sane).
- Post: `resolve` never performs drop-store I/O (metadata only).
- Errors: `UnsupportedFeature`, `Corrupt`, `Policy` (overlay depth/cycle).

### I2 — DropSource (read a drop's plaintext)

```rust
trait DropSource {
    fn read_drop(&self, id: DropId, rep: &Representation, dst: &mut [u8])
        -> Result<(), CoreError>;
    fn read_range(&self, slab: SlabRef, range: Range<u64>) -> Result<Bytes>;
}
```

- Post-condition (the load-bearing one): returned bytes hash to `id` under
  BLAKE3. Verification is *inside* this interface, not the caller's job.
- `dst` sizing comes from metadata (`DropId → plaintext len`); no caller
  re-buffering (zero-copy/zero-realloc rule).
- Errors: `Integrity` (hash mismatch — hard fail, never yield), `Locator`,
  `Authentication` (AEAD open failed).

### I3 — Codec (registry)

```rust
trait Codec { fn id(&self) -> CodecId; fn decode(&self, src: &[u8], dst: &mut [u8]) -> Result<usize>;
              fn encode(&self, src: &[u8], level: u8, dst: &mut Vec<u8>) -> Result<()>; }
```
- Builtin IDs: 0x00 store, 0x01 lz4, 0x02 zstd; registry rows for lzma/brotli.
- Implementations must be deterministic (same input+level ⇒ same bytes) —
  required for reproducible images and conformance vectors.

### I4 — Crypto traits (05)

```rust
trait Aead    { fn id(&self) -> AeadId; fn seal(&self, k:&Key, n:&Nonce, ad:&[u8], pt:&[u8]) -> Vec<u8>;
                fn open(&self, k:&Key, n:&Nonce, ad:&[u8], ct:&[u8]) -> Result<Vec<u8>, CryptoError>; }
trait KeyWrap { fn wrap(&self, image_key:&Key, to:&PublicKey) -> Envelope;
                fn unwrap(&self, env:&Envelope, sk:&SecretKey) -> Result<Key>; }
trait Signer  { fn sign(&self, root: ManifestRoot, policy:&PolicySection) -> SignatureBundle;
                fn verify(bundle:&SignatureBundle, root: ManifestRoot) -> Result<Identity>; }
trait Dms     { fn seal(&self, key:&Key, policy:&DmsPolicy) -> DmsRecord;
                fn solve(&self, rec:&DmsRecord, shares:&[Share]) -> Result<Key>; }
```

- Nonce/AD construction is *not* the trait's caller's choice: `Nonce::derive(key, slab, idx)`
  and `AssociatedData::new(root, slab, idx)` are fixed functions (02-algorithms §5).
- 03 and 04 depend only on these traits; algorithm crates plug into 05's registry.

### I5 — Classifier (04)

```rust
trait Classifier { fn id(&self) -> ClassifierId;
                   fn classify(&self, head: &[u8], features: Features) -> Option<ClassId>; }
```
- Chain-of-responsibility: registry ordered; first `Some` wins; fallback class `binary`.
- Contract: classification affects *ratio only*. Any misclassification must
  still round-trip (conformance vector enforces).

### I6 — Sink (write side storage)

```rust
trait Sink {
    fn put_drop(&mut self, id: DropId, rep: Representation, bytes: &[u8]) -> Result<SlabRef>;
    fn seal_slab(&mut self) -> Result<SlabId>;       // close current slab
    fn commit(self, manifest: Manifest) -> Result<()>; // atomic publish
}
```
- Atomicity: readers see either the old manifest or the new one, never a mix.
  Implementations: write-then-rename (file), multipart+conditional PUT (s3).
- Cancel-safety: abandoning a `Sink` before `commit` leaves no reader-visible state.

### I7 — Delta/merge (06)

```rust
trait DeltaBuilder { fn diff(&self, base:&Tree, next:&Tree, sink:&mut dyn Sink) -> Result<DeltaManifest>; }
trait Flattener    { fn flatten(&self, chain:&[ManifestRef]) -> Result<Manifest>; } // zero drop-store I/O
trait Turnover     { fn turnover(&self, chain:&[ManifestRef], sink:&mut dyn Sink) -> Result<Manifest>; }
```
- Flatten post-condition: resulting tree resolves byte-identical to the chain;
  no drop-store bytes read or written (asserted in tests).
- Turnover post-condition: standalone manifest (no external slab refs);
  `history` records operation kind (flatten vs turnover).

### I8 — Ec (07)

```rust
trait Ec { fn id(&self) -> EcId; fn encode(&self, slab:&[u8], k:u8, m:u8) -> Vec<Shard>;
           fn reconstruct(&self, shards:&[Option<Shard>], k:u8) -> Result<Vec<u8>, EcError>; }
```
- Reconstruction verifies against the slab hash before yielding (EC =
  availability; hash = integrity).

### I9 — Locator (08)

```rust
trait Locator {
    fn scheme(&self) -> SchemeId;
    fn get(&self, r:&SlabRef, range:Range<u64>) -> BoxStream<Result<Bytes, LocatorError>>;
    fn put(&self, slab:&[u8]) -> Result<LocatorEntry>;   // writer path
}
struct LocatorRegistry { /* SchemeId → factory; per-slab mirror list; racing policy */ }
```
- Streaming contract: first byte without full-slab transfer (except recorded
  solid blocks); mirrors raced, liars demoted (hash mismatch propagates as
  `Integrity` from I2 above the locator).

### I10 — legacy adapter (09)

- Implements I1+I2 for Frozen2 images. No write traits exist for it. Errors
  for unsupported legacy features map to `Unsupported`.

### I11 — libtfs adapter (12, external contract)

- LimniFS plugs into libtfs's `FileSystem` interface (`include/tebako/fs/filesystem.h`)
  like any backend; adapter translates libtfs calls → I1/I2. No libtfs
  modification permitted — the adapter proves the OCP story to an outside system.

### I12 — conformance protocol (02)

- Black-box: harness feeds fixture bytes + operation script (`open`, `resolve
  path`, `read range`, `verify`, `walk`) and compares outputs + identities.
- Implementations expose a thin stdin/stdout driver; suite never links impl code.

### I13 — codegen artifacts (01)

- All crates consume generated bindings + registry tables; none redefine wire
  constants locally. CI diffs generated output against committed code.

## 4. Call sequences (normative flows)

### 4.1 Read via mount (hot path)

```
FUSE(11)     core(03)        crypto(05)   locator(08)     EC(07)
  │ read(path,off,len)          │            │              │
  │──────────────►│             │            │              │
  │               │ resolve → slice_map      │              │
  │               │ per drop:  │             │              │
  │               │  slab index→SlabRef      │              │
  │               │────────────┼────────────►│ get(range)   │
  │               │            │             │──(miss?)─────►│reconstruct
  │               │  AEAD open │             │  bytes        │
  │               │◄───────────┤             │              │
  │               │ BLAKE3 == DropId (I2 post-condition)     │
  │◄──────────────│ plaintext  │             │              │
```

### 4.2 Build (limn)

```
cli(10) → writer(04): for each slice:
  FastCDC → drops → seine classify → dedup check (DropId in index?)
  → new drops: LZ4/store → Sink.put_drop → slab packing
  → seal slabs → assemble metadata+manifest → Sink.commit (atomic)
  → print ManifestRoot
```

### 4.3 Deepen (background)

```
writer(04): select drops by policy (age/class/tier)
  → re-encode per class (zstd/lzma/brotli)
  → append Representation rows (identity untouched)
  → optional repack via new slabs → new manifest (atomic swap)
```

### 4.4 Delta + flatten + turnover

```
06: diff(base,next) → delta manifest (base_root) + new drops via 04
06: flatten(chain)  → metadata merge ONLY → composite manifest (zero data I/O)
06: turnover(chain) → walk resolved tree → re-pack all drops via 04 Sink
                    → GC unreferenced → standalone manifest
```

### 4.5 Verify (limni verify)

```
10 → 03: walk tree, read every drop (I2 post-condition verifies hashes)
   → 05: AEAD tag check per sealed drop; signature bundle vs ManifestRoot
   → report: proven{ drops, aead, signature, chain linkage } or first failure + ID
```

## 5. Observability contract

- Every boundary crossing above emits a `tracing` span named
  `limnifs::{module}::{operation}` with semantic IDs as fields; no logging of
  plaintext or keys, ever.
- Counters (registry-defined names): drops read/written, bytes per tier,
  locator RTT histogram, EC repairs, cache hits. Emitted via traits so
  embedded consumers (tebako) can substitute their own sink.
