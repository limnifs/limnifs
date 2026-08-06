# 21 — Drop-record batch encoding

- **Priority:** P2
- **Side:** LimniFS
- **Est. effort:** 3h

## Problem

`encode_slab` builds the drop-records section via per-drop
`extend_from_slice` calls in a loop:

```rust
for drop in drops {
    drop_records.extend_from_slice(&drop.id);
    drop_records.extend_from_slice(&plaintext_len.to_le_bytes());
    drop_records.extend_from_slice(&[drop.codec, 0x00, 0x00]);
    // ... 6 more extend_from_slice calls
}
```

Each `extend_from_slice` is a single memcpy + bounds check. For 1M
drops, that's 9M method calls. Most drop records are exactly 49
bytes — fixed-size.

## Fix

Pre-allocate `drop_records: Vec<u8>` with `drops.len() * 49`, then
write each record into the slot directly via `write_all` or unsafe
indexing. Or: build per-record `[u8; 49]` then `extend_from_slice`
the fixed-size array.

## Expected impact

- 5–10% on tiny-files (50K drops)
- 2–5% on container images (100K drops)
- Negligible on small images

## Acceptance

- [ ] Drop records batched via fixed-size writes
- [ ] Output bytes unchanged
- [ ] Benchmark: tiny-files improves
