# 02 — Algorithm specifications

Normative algorithms. Conformance vectors (02-conformance) are generated to
exercise every boundary condition stated here. Pseudocode is binding on
*behavior* (outputs, bounds), not on internal implementation style.

## 1. FastCDC content-defined chunking (04)

Goal: split a slice into drops whose boundaries depend on local content, so
insertions shift ≤ 2 boundaries (dedup stability).

```
params: min = 64 KiB, avg = 256 KiB, max = 1 MiB   (per-image, in manifest)
        gear table G[256] (fixed, spec-defined), mask M = (1<<ceil(log2(avg)))-1
state:  h = 0
for each byte b at position p:
    h = (h << 1) + G[b]            // rolling gear hash, u64 wrapping
    if p - chunk_start >= min and (h & M) == 0: cut boundary
    if p - chunk_start >= max: cut boundary (forced)
emit drop [chunk_start, p)
```

- Normalization: between min and max, use two masks (small mask below avg,
  large mask above) per FastCDC's normalized chunking — boundary distribution
  tightens around avg.
- Complexity: O(n) time, O(1) space, single pass, streaming (`impl Read`).
- Boundary conditions (vectors required): empty slice (one zero-length drop);
  slice < min (single drop); all-zero input (forced max cuts only);
  1-byte insert at k ⇒ ≤ 2 changed boundaries; boundary at exactly min/max.
- Parameters live in the manifest; changing them never forks the format
  (reader is parameter-agnostic — chunking is a write-side concern only).

## 2. Identity hashing (BLAKE3, 01/03)

- `DropId = BLAKE3(plaintext)` — full 256-bit output, base32 multihash form `b3:…` for display.
- Why BLAKE3: tree-mode parallelism (≥ 2 GB/s/core), keyed/derive modes used by §5,
  constant-time, single dependency, no length-extension.
- Vectors: empty input, 1 byte, exactly chunk-of-tree boundary (1024-byte,
  8 MiB subtree edges), > 4 GiB drop (streaming hash, bounded memory).

## 3. Merkle tree over the image (01/03)

`ManifestRoot` commits to: fs metadata buffer, slab index, crypto/EC/DMS
parameter sections, delta linkage.

```
root = BLAKE3( "limnifs/v1" || H(metadata) || H(slab_index) || H(crypto_params)
            || H(ec_params) || H(dms_policy) || base_root? )
```

- Domain separator string prevents cross-protocol confusion.
- Flat construction (hash of section hashes), not a deep tree: sections are
  individually verifiable without loading siblings; a lying locator on one
  section is isolated.
- Vectors: section substitution (swap metadata of another image ⇒ root
  mismatch), section omission, base_root transplant.

## 4. Seine classification (04)

Ordered classifier chain; first match wins; fallback `binary`.

```
features(drop): entropy8(head 4 KiB), magic(head 16), nul_ratio, printable_ratio
class rules (v1, registry-extensible):
  already-compressed : magic ∈ {gzip,xz,zstd,lzma,zip,png,jpg,mp4,…} or entropy8 ≥ 7.99
  sparse             : nul_ratio ≥ 0.99 over full drop
  text/code          : printable_ratio ≥ 0.95 and nul_ratio ≈ 0
  media              : magic ∈ {wav,flac,raw image,…} (compressible media)
  binary             : fallback
```

- Contract: classification affects ratio only — any class round-trips.
- Per-class deep-codec mapping (hypolimnion): text/code → lzma|brotli,
  binary → zstd-19, compressed → store, sparse → store + sparse flag,
  media → zstd-19. Registry data, not code.

## 5. AEAD application (05)

```
key:    image_key (32 B, random per image; wrapped per recipient, §6)
nonce:  Nonce::derive = HKDF-BLAKE3(image_key, info = slab_id ‖ u64le(drop_index))[0..24]
ad:     AssociatedData = manifest_root ‖ slab_id ‖ u64le(drop_index)
seal:   ct = AEAD[aead_id].seal(image_key, nonce, ad, plaintext)
open:   pt = AEAD[aead_id].open(...) ; verify BLAKE3(pt) == drop_id   (defense in depth)
```

- Deterministic nonces are safe here *because* data is immutable: (key, slab,
  index) tuples are unique by construction. Mutable data would forbid this.
- Transplant resistance: moving ct to another slab/index/image changes AD ⇒
  open fails. Vectors: transplant same-slab, cross-slab, cross-image.
- Algorithm registry: 0x01 XChaCha20-Poly1305 (mandatory), 0x02 AES-128-OCB
  (AES-NI fast path, RFC 7253, patents abandoned 2021), 0x03 AES-256-GCM,
  0x04 Ascon-128a (embedded readers).

## 6. Key wrapping (HPKE, 05)

```
per recipient i:  envelope_i = HPKE-Base(DHKEM-X25519, HKDF-SHA256, ChaChaPoly)(pk_i, image_key)
manifest carries  { recipient_key_id: envelope_i }
```

- Add recipient: append envelope (image key unchanged, drops untouched).
- Remove recipient: drop envelope; residual access is a stated threat-model
  property (prior readers retain the key — re-key = new image version).

## 7. Reed-Solomon layout (07)

```
per slab S (bytes): strip S into k data shards of ceil(|S|/k) bytes (zero-padded tail)
parity: m shards via RS(k+m) over GF(2^8), polynomial per spec table (0x11D)
shard records: { slab_id, shard_index ∈ [0, k+m), hash BLAKE3(shard), locator_entry }
reconstruct(shards): any k ⇒ Berlekamp-Welch / Cauchy-matrix inversion
  → verify BLAKE3(join(data shards)) == slab_hash before yield
```

- Complexity: encode O(n·m) per slab word-parallel (SIMD); reconstruct O(k²) matrix
  inversion once + O(n·k) apply.
- Vectors: lose each single shard, lose m shards (all C(k+m, m) for k=4,m=2),
  corrupt one shard (hash catches), k−1 shards ⇒ clean error.
- Extensibility: `Ec` trait admits fountain/LT codes later (registry row).

## 8. Delta diff (06)

Tree diff between base tree B and next tree N, emitting minimal ops:

```
walk(B, N) in lockstep by (path):
  inode in both, same content ids  → no op
  inode in both, changed           → replace(path, new slice_map, new attrs)
  inode in N only                  → add(path, …)   (new drops via 04 pipeline)
  inode in B only                  → remove(path)
  rename detection: identical content id sets in add/remove pairs ⇒ rename(from,to)
```

- Rename semantics per spec decision (01-spec resolves design §16.4 before
  implementation): either first-class `rename` ops or compile to remove+add —
  one behavior, spec-mandated, not implementation whim.
- Identity: inodes carry content-derived ids (hash of slice-map) so rename
  detection is O(1) per pair via hash map, not O(n²).
- Complexity: O(|B| + |N|) metadata, plus write cost of genuinely new content.

## 9. Flatten (tier 2, 06)

```
flatten(chain [m0 (base) … mn]):
  merged = copy(m0.manifest)
  for mi in m1…mn: apply mi.tree_ops onto merged metadata
  slab_index = union of all referenced slabs across chain (locator entries carried)
  history = append {op: flatten, inputs: [roots…]}
```

- Zero drop-store I/O: reads only manifests/metadata. Asserted by test that
  fails on any slab fetch.
- Post-condition: `resolve(flatten(chain)) == resolve(chain)` byte-identical
  (conformance vector).
- Complexity: O(total metadata of chain). No data movement ⇒ seconds at GB scale.

## 10. Turnover (tier 3, 06+04)

```
turnover(chain):
  tree = resolve(chain)
  sink = new Sink
  for slice in tree:                 // streaming, bounded memory
      for drop_id in slice_map:      // referenced drops only
          bytes = read_drop(drop_id) // from chain via 03
          sink.put_drop(drop_id, chosen_rep(chain), bytes)  // re-pack, re-deepen per policy
  manifest = assemble(tree.metadata, sink.slabs)
  history = append {op: turnover, inputs: [roots…]}
  commit atomically
```

- GC is implicit and exact: anything unreachable from the resolved tree is
  never copied. Mark-and-sweep == copy-the-live-set.
- Post-conditions: standalone (no external slab refs), byte-identical tree,
  cancel-safe (old image valid until commit).
- Complexity: O(live data) I/O — the only tier that moves bytes.

## 11. DMS primitives (05)

**Shamir k-of-n escrow (v1):**
```
split(key): per key byte, random poly f(x) of degree k−1 over GF(2^8), f(0)=key_byte
            share_i = (i, f(i)) ; n shares, any k reconstruct via Lagrange at x=0
```
- Vectors: all C(n,k) for n≤5, k−1 ⇒ no information (information-theoretic),
  share metadata survives flatten/turnover (carried in DMS policy section).

**Time-lock puzzle (post-v1, gated on design §16.3):**
```
seal(key, T):  puzzle = (N = p·q, a, t) ; encapsulate key with a^(2^t) mod N
solve:         t iterated squarings — sequential, parallelization-resistant
```
- Calibration problem (hardware drift between seal and solve) is unresolved;
  v1 ships Shamir only. Puzzle code must not land until the spec defines
  parameter selection and a Wesolowski/Pietrzak proof-of-elapsed-time scheme.

## 12. Determinism requirements (cross-cutting)

For reproducible images and conformance: chunking, classification, codec
encode(at fixed level), slab packing order (slice traversal order, spec-fixed),
and manifest assembly are all deterministic functions of (input tree,
parameters). Timestamps/randomness enter only via explicit parameters
(image creation time, image key). Any nondeterminism found in CI (same input,
two runs, different ManifestRoot) is a bug class of its own.
