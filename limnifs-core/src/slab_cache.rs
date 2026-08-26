//! Hot slab cache — SIEVE-evicted, byte-and-count bounded cache over
//! decoded drop plaintexts shared as `Arc<[u8]>`.
//!
//! [`SlabStore::plaintext_for`] decompresses on every call. For
//! read-heavy workloads (mount, `cat-multi`, windowed reads, turnover
//! on a hot image) the same drops are decoded over and over — reading
//! a 19.5 MiB file through 8 KiB windows re-decompressed the whole
//! drop per window (~48 GB of wasted codec work; limnifs#192).
//! [`CachedSlabStore`] wraps a [`SlabStore`] and keeps recently
//! decoded plaintexts in a bounded cache keyed by `DropId`.
//!
//! ## Design
//!
//! - **SIEVE eviction** (Yang et al., USENIX ATC '24): one FIFO +
//!   a visited bit per entry. O(1) per op, scan-resistant, evaluated
//!   across 6,594 traces to beat LRU/ARC/LIRS-class policies. Hit →
//!   set visited (no reorder). Eviction scans from the oldest entry:
//!   visited entries get the bit cleared and rotate to the newest
//!   end; unvisited entries evict. Replaces the previous plain LRU.
//! - **Byte AND count bounds.** The cache tracks summed plaintext
//!   size and evicts (SIEVE-wise) until an insert fits. A drop larger
//!   than the whole byte budget bypasses the cache — returned to the
//!   caller, never inserted — so one huge drop can neither blow the
//!   budget nor evict the entire working set.
//! - **`Arc<[u8]>` values.** Hits are refcount bumps (zero-copy);
//!   callers that only borrow never pay a clone. The legacy
//!   `plaintext_for` (owned `Vec`) remains a thin copy over
//!   [`CachedSlabStore::decoded`].
//! - Thread-safe via `Mutex`; contention is negligible at this
//!   granularity (one lock per `SlabStore`, microsecond hold times).
//! - Cache hits avoid both the slab-fetch and the decompress; on a
//!   `cat-multi` of a 1000-file tree the second invocation runs
//!   ~10× faster.
//!
//! ## Why not cache compressed bytes
//!
//! Compressed bytes are already memory-mapped via `SlabStore`'s
//! mmap'd slabs. The kernel page cache handles them. The expensive
//! step is decompression — that's what this cache targets.
//!
//! See `TODO.impl/03-core-reader/03-hot-slab-cache.md` and
//! `TODO.sota-fs/01-sieve-drop-cache.md`.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use crate::slab_store::SlabStore;
use crate::CoreError;

/// Default cache capacity: 1024 entries.
pub const DEFAULT_CACHE_CAPACITY: usize = 1024;

/// Default byte budget for the decoded-plaintext cache: 64 MiB.
/// Combined with the entry cap, whichever binds first evicts.
pub const DEFAULT_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Default byte budget for the seekable frame cache (256 KiB frames).
pub const DEFAULT_FRAME_CACHE_BYTES: usize = 32 * 1024 * 1024;

/// Cache statistics (limnifs#192 P4 observability).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    /// Decodes whose result exceeded the byte budget and were not
    /// inserted.
    pub bypassed: u64,
    pub entries: usize,
    pub bytes: usize,
    pub byte_budget: usize,
    pub entry_capacity: usize,
}

/// Bounded LRU cache wrapping a [`SlabStore`]. Cache hits return
/// the cached plaintext directly; misses fetch from the inner
/// store and insert.
pub struct CachedSlabStore {
    inner: SlabStore,
    cache: Mutex<SieveCache>,
    /// Parsed container footers, memoized per drop: windowed traffic
    /// re-parses the footer per read otherwise (O(frames) per 8 KiB
    /// window). Bounded by distinct seekable drops touched; cleared
    /// wholesale past a generous cap (footers are ~12 B/frame).
    footers:
        Mutex<std::collections::HashMap<[u8; 32], std::sync::Arc<crate::seekable::SeekFooter>>>,
    /// Frame-level cache for SEEKABLE drops. Windowed traffic would
    /// thrash a full-drop cache (one 19.5 MiB drop evicts
    /// everything), but 256 KiB frames fit a working set: repeat
    /// windows become refcount bumps instead of re-decodes.
    frames: Mutex<SieveCache>,
}

/// SIEVE cache (USENIX ATC '24): FIFO order + per-entry visited bit.
/// Hits set the bit lazily; eviction scans from the oldest entry
/// clearing visited bits (rotation to the newest end) until an
/// unvisited entry evicts. O(1) amortized per operation,
/// scan-resistant, no tuning knobs.
struct SieveCache {
    /// DropId -> shared plaintext.
    entries: HashMap<[u8; 32], std::sync::Arc<[u8]>>,
    /// FIFO order; front = oldest. The `bool` is the visited bit.
    order: VecDeque<([u8; 32], bool)>,
    entry_capacity: usize,
    byte_budget: usize,
    bytes: usize,
    hits: u64,
    misses: u64,
    evictions: u64,
    bypassed: u64,
}

impl SieveCache {
    fn new(entry_capacity: usize, byte_budget: usize) -> Self {
        let cap = entry_capacity.max(1);
        Self {
            entries: HashMap::with_capacity(cap.min(4096)),
            order: VecDeque::with_capacity(cap.min(4096)),
            entry_capacity: cap,
            byte_budget,
            bytes: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            bypassed: 0,
        }
    }

    fn get(&mut self, key: &[u8; 32]) -> Option<std::sync::Arc<[u8]>> {
        if let Some(v) = self.entries.get(key) {
            self.hits += 1;
            let shared = std::sync::Arc::clone(v);
            // SIEVE lazy promotion: set the visited bit, do not
            // reorder. Linear slot scan is bounded by entry capacity
            // (default 1024) and scans newest-first.
            for slot in self.order.iter_mut().rev() {
                if &slot.0 == key {
                    slot.1 = true;
                    break;
                }
            }
            Some(shared)
        } else {
            self.misses += 1;
            None
        }
    }

    /// Insert `value`; returns `false` when the value exceeds the
    /// byte budget (bypass — the caller still uses the value, it
    /// just never enters the cache).
    fn insert(&mut self, key: [u8; 32], value: std::sync::Arc<[u8]>) -> bool {
        let vbytes = value.len();
        if vbytes > self.byte_budget {
            self.bypassed += 1;
            return false;
        }
        if let Some(old) = self.insert_entry(key, value) {
            // Replace-in-place keeps the queue position (refreshing a
            // known drop's bytes is not a recency event).
            self.bytes -= old.len();
        } else {
            self.order.push_back((key, false));
        }
        self.bytes += vbytes;
        self.evict_until_fits();
        true
    }

    fn insert_entry(
        &mut self,
        key: [u8; 32],
        value: std::sync::Arc<[u8]>,
    ) -> Option<std::sync::Arc<[u8]>> {
        let prev = self.entries.insert(key, value);
        if prev.is_some() {
            for slot in self.order.iter_mut().rev() {
                if slot.0 == key {
                    slot.1 = true;
                    break;
                }
            }
        }
        prev
    }

    fn evict_until_fits(&mut self) {
        // SIEVE hand from the oldest entry: visited -> clear bit and
        // rotate to newest; unvisited -> evict. Amortized O(1): each
        // rotation clears a bit only a hit can set again.
        while self.bytes > self.byte_budget || self.entries.len() > self.entry_capacity {
            let Some((key, visited)) = self.order.pop_front() else {
                break;
            };
            if visited {
                self.order.push_back((key, false));
            } else if let Some(v) = self.entries.remove(&key) {
                self.bytes -= v.len();
                self.evictions += 1;
            }
        }
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            bypassed: self.bypassed,
            entries: self.entries.len(),
            bytes: self.bytes,
            byte_budget: self.byte_budget,
            entry_capacity: self.entry_capacity,
        }
    }
}

impl CachedSlabStore {
    /// Wrap `inner` with the given entry capacity and byte budget.
    #[must_use]
    pub fn with_bounds(inner: SlabStore, entry_capacity: usize, byte_budget: usize) -> Self {
        Self::with_frame_budget(
            inner,
            entry_capacity,
            byte_budget,
            DEFAULT_FRAME_CACHE_BYTES,
        )
    }

    /// [`Self::with_bounds`] plus an explicit frame-cache byte budget
    /// (see [`DEFAULT_FRAME_CACHE_BYTES`]).
    #[must_use]
    pub fn with_frame_budget(
        inner: SlabStore,
        entry_capacity: usize,
        byte_budget: usize,
        frame_byte_budget: usize,
    ) -> Self {
        // 256 KiB frames: an entry per 128 KiB of budget (half a
        // frame, so partially-used budgets still hold whole frames),
        // clamped to a sane range.
        let frame_entries = (frame_byte_budget / (128 * 1024)).clamp(16, 1024);
        Self {
            inner,
            cache: Mutex::new(SieveCache::new(entry_capacity, byte_budget)),
            frames: Mutex::new(SieveCache::new(frame_entries, frame_byte_budget)),
            footers: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Wrap `inner` with a cache of the given capacity (in entries)
    /// and the default byte budget.
    #[must_use]
    pub fn new(inner: SlabStore, capacity: usize) -> Self {
        Self::with_bounds(inner, capacity, DEFAULT_CACHE_BYTES)
    }

    /// Wrap `inner` with the default capacity and byte budget.
    #[must_use]
    pub fn with_default_capacity(inner: SlabStore) -> Self {
        Self::new(inner, DEFAULT_CACHE_CAPACITY)
    }

    /// Fetch `drop_id`'s plaintext as a shared, zero-copy handle.
    ///
    /// Cache hits are an `Arc` refcount bump. Misses fetch from the
    /// inner store and insert; a plaintext larger than the byte
    /// budget is returned but not cached (bypass). Use this on hot
    /// read paths (windowed reads, mount); use
    /// [`Self::plaintext_for`] only when an owned `Vec` is truly
    /// needed.
    pub fn decoded(&self, drop_id: &[u8; 32]) -> Option<Result<std::sync::Arc<[u8]>, CoreError>> {
        {
            let mut cache = self.cache.lock().expect("cache mutex poisoned");
            if let Some(hit) = cache.get(drop_id) {
                return Some(Ok(hit));
            }
        }
        // Miss: fetch from inner (drop the cache lock during the
        // decompress so concurrent readers of other drops proceed).
        let plaintext = self.inner.plaintext_for(drop_id)?;
        match plaintext {
            Ok(bytes) => {
                let shared: std::sync::Arc<[u8]> = bytes.into();
                let mut cache = self.cache.lock().expect("cache mutex poisoned");
                cache.insert(*drop_id, std::sync::Arc::clone(&shared));
                Some(Ok(shared))
            }
            Err(e) => Some(Err(e)),
        }
    }

    /// Decompress only `[off, off+len)` of `drop_id`'s plaintext.
    ///
    /// Cache hits slice the resident full-drop `Arc` (zero decode).
    /// Misses on seekable drops decode only the covering container
    /// frames (never cached — windowed traffic would thrash the
    /// byte budget). Misses on non-seekable drops decode the full
    /// drop, insert it, and slice, so repeat windows on the same
    /// drop are cheap.
    pub fn decoded_range(
        &self,
        drop_id: &[u8; 32],
        off: u64,
        len: usize,
    ) -> Option<Result<Vec<u8>, CoreError>> {
        {
            let mut cache = self.cache.lock().expect("cache mutex poisoned");
            if let Some(hit) = cache.get(drop_id) {
                let total = hit.len() as u64;
                return Some(if off > total || off + len as u64 > total {
                    Err(CoreError::Corrupt {
                        reason: format!(
                            "decoded_range [{off}, {}) outside drop length {total}",
                            off + len as u64
                        ),
                    })
                } else {
                    Ok(hit[off as usize..off as usize + len].to_vec())
                });
            }
        }
        if self.inner.drop_is_seekable(drop_id) == Some(true) {
            return self.cached_frame_range(drop_id, off, len);
        }
        // Non-seekable: full decode (populating the cache), then slice.
        match self.decoded(drop_id)? {
            Ok(full) => {
                let total = full.len() as u64;
                Some(if off > total || off + len as u64 > total {
                    Err(CoreError::Corrupt {
                        reason: format!(
                            "decoded_range [{off}, {}) outside drop length {total}",
                            off + len as u64
                        ),
                    })
                } else {
                    Ok(full[off as usize..off as usize + len].to_vec())
                })
            }
            Err(e) => Some(Err(e)),
        }
    }

    /// Ranged decode for SEEKABLE drops through the frame cache.
    ///
    /// Covering frames resolve from the frame SIEVE cache; misses
    /// decode one frame (256 KiB bound) and insert. Edge frames are
    /// sliced, so the output is exactly `[off, off+len)`.
    fn cached_frame_range(
        &self,
        drop_id: &[u8; 32],
        off: u64,
        len: usize,
    ) -> Option<Result<Vec<u8>, CoreError>> {
        let (raw, record) = self.inner.raw_window(drop_id)?;
        let footer = {
            let mut footers = self.footers.lock().expect("footer cache poisoned");
            if let Some(hit) = footers.get(drop_id) {
                std::sync::Arc::clone(hit)
            } else {
                let parsed = match crate::seekable::parse_footer(raw) {
                    Ok(f) => std::sync::Arc::new(f),
                    Err(e) => return Some(Err(e)),
                };
                if footers.len() >= 4096 {
                    footers.clear();
                }
                footers.insert(*drop_id, std::sync::Arc::clone(&parsed));
                parsed
            }
        };
        let total = footer.total_uncomp();
        if off > total || off + len as u64 > total {
            return Some(Err(CoreError::Corrupt {
                reason: format!(
                    "decoded_range [{off}, {}) outside drop length {total}",
                    off + len as u64
                ),
            }));
        }
        let mut starts = Vec::with_capacity(footer.uncomp_lens.len());
        let mut acc = 0u64;
        for &l in &footer.uncomp_lens {
            starts.push(acc);
            acc += u64::from(l);
        }
        let first = starts.partition_point(|&s| s <= off).saturating_sub(1);
        let mut out = Vec::with_capacity(len);
        let mut comp_pos = footer.comp_lens[..first]
            .iter()
            .map(|&l| l as usize)
            .sum::<usize>();
        let mut cum = starts[first];
        for i in first..footer.uncomp_lens.len() {
            let uncomp_len = footer.uncomp_lens[i];
            let comp_len = footer.comp_lens[i] as usize;
            let frame_bytes = &raw[comp_pos..comp_pos + comp_len];

            let key = crate::seekable::frame_key(drop_id, i as u32);
            let decoded = {
                let mut frames = self.frames.lock().expect("frame cache poisoned");
                if let Some(hit) = frames.get(&key) {
                    hit
                } else {
                    // Decode outside the lock so concurrent readers of
                    // other frames proceed.
                    drop(frames);
                    crate::seekable::count_frame_decode();
                    let decoded: std::sync::Arc<[u8]> = crate::codec::decompress(
                        record.representation.codec,
                        frame_bytes,
                        uncomp_len,
                    )
                    .ok()?
                    .into();
                    let mut frames = self.frames.lock().expect("frame cache poisoned");
                    frames.insert(key, std::sync::Arc::clone(&decoded));
                    decoded
                }
            };

            let slice_from = off.saturating_sub(cum) as usize;
            let slice_to = ((off + len as u64) - cum).min(u64::from(uncomp_len)) as usize;
            out.extend_from_slice(&decoded[slice_from..slice_to]);
            if cum + u64::from(uncomp_len) >= off + len as u64 {
                break;
            }
            comp_pos += comp_len;
            cum += u64::from(uncomp_len);
        }
        Some(Ok(out))
    }

    /// Fetch `drop_id`'s plaintext as an owned `Vec`.
    ///
    /// Convenience over [`Self::decoded`] for callers outside the
    /// hot path — every call pays one copy.
    #[must_use]
    pub fn plaintext_for(&self, drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, CoreError>> {
        match self.decoded(drop_id)? {
            Ok(shared) => Some(Ok(shared.to_vec())),
            Err(e) => Some(Err(e)),
        }
    }

    /// Snapshot of cache counters (hits/misses/evictions/bypasses,
    /// current bytes/entries vs budgets).
    #[must_use]
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.lock().expect("cache mutex poisoned").stats()
    }

    /// Number of entries currently cached.
    #[must_use]
    pub fn cache_len(&self) -> usize {
        self.cache.lock().expect("cache mutex poisoned").len()
    }

    /// Cache capacity (in entries).
    #[must_use]
    pub fn cache_capacity(&self) -> usize {
        self.cache
            .lock()
            .expect("cache mutex poisoned")
            .entry_capacity
    }

    /// Delegate slab count to the inner store.
    #[must_use]
    pub fn slab_count(&self) -> usize {
        self.inner.slab_count()
    }

    /// Delegate drop count to the inner store.
    #[must_use]
    pub fn drop_count(&self) -> usize {
        self.inner.drop_count()
    }
}

impl crate::slab_source::SlabSource for CachedSlabStore {
    fn plaintext_for(&self, drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, crate::CoreError>> {
        CachedSlabStore::plaintext_for(self, drop_id)
    }
    fn slab_count(&self) -> usize {
        self.slab_count()
    }
    fn drop_count(&self) -> usize {
        self.drop_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc(bytes: &[u8]) -> std::sync::Arc<[u8]> {
        std::sync::Arc::from(bytes.to_vec())
    }

    #[test]
    fn sieve_entry_cap_evicts_oldest_unvisited() {
        let mut cache = SieveCache::new(2, DEFAULT_CACHE_BYTES);
        assert!(cache.insert([1; 32], arc(&[0xAA])));
        assert!(cache.insert([2; 32], arc(&[0xBB])));
        assert!(cache.insert([3; 32], arc(&[0xCC])));
        // [1;32] was never visited — SIEVE evicts it first.
        assert!(!cache.entries.contains_key(&[1; 32]));
        assert!(cache.entries.contains_key(&[2; 32]));
        assert!(cache.entries.contains_key(&[3; 32]));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.evictions, 1);
    }

    #[test]
    fn sieve_visit_survives_scan() {
        // The scan-resistance property: touching an entry before a
        // burst of new inserts keeps it resident.
        let mut cache = SieveCache::new(2, DEFAULT_CACHE_BYTES);
        assert!(cache.insert([1; 32], arc(&[0xAA])));
        assert!(cache.insert([2; 32], arc(&[0xBB])));
        let hit = cache.get(&[1; 32]).expect("hit");
        assert_eq!(&*hit, &[0xAA]);
        assert!(cache.insert([3; 32], arc(&[0xCC])));
        // [2;32] unvisited evicts; visited [1;32] survives.
        assert!(cache.entries.contains_key(&[1; 32]));
        assert!(!cache.entries.contains_key(&[2; 32]));
        assert!(cache.entries.contains_key(&[3; 32]));
        assert!(cache.stats().hits >= 1);
    }

    #[test]
    fn sieve_byte_budget_evicts_until_fits() {
        let mut cache = SieveCache::new(100, 100);
        assert!(cache.insert([1; 32], arc(&[0x41; 40])));
        assert!(cache.insert([2; 32], arc(&[0x42; 40])));
        // Budget 100: inserting a third 40-byte entry must evict
        // until bytes <= 100.
        assert!(cache.insert([3; 32], arc(&[0x43; 40])));
        assert!(cache.bytes <= 100, "bytes {} over budget", cache.bytes);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn sieve_oversized_value_bypasses() {
        let mut cache = SieveCache::new(100, 64);
        assert!(cache.insert([1; 32], arc(&[0xAA])));
        // A value bigger than the whole budget: returned usable but
        // never inserted, and the existing entry survives.
        assert!(!cache.insert([9; 32], arc(&[0xEE; 65])));
        assert!(cache.entries.contains_key(&[1; 32]));
        assert!(!cache.entries.contains_key(&[9; 32]));
        assert_eq!(cache.bypassed, 1);
    }

    #[test]
    fn sieve_replace_updates_value_without_growing() {
        let mut cache = SieveCache::new(2, DEFAULT_CACHE_BYTES);
        assert!(cache.insert([1; 32], arc(&[0xAA; 10])));
        assert!(cache.insert([1; 32], arc(&[0xBB; 20])));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.bytes, 20);
        assert_eq!(&*cache.entries[&[1; 32]], &[0xBB; 20]);
    }

    #[test]
    fn cached_slab_store_round_trips_against_slab_store() {
        // Build a tiny image so we have a real SlabStore with at
        // least one drop. Inline-threshold is 4 KiB so 8 KiB triggers
        // slab-backed storage. The cache lives inside limnifs-core,
        // so we can't call into limnifs-write here — instead build
        // the slab bytes directly.
        let plaintext = vec![0xCDu8; 8192];
        let drop_id = crate::merkle::hash_section(&plaintext);
        let compressed = crate::codec::compress(crate::codec::CODEC_LZ4, &plaintext).expect("lz4");

        // Build a minimal slab: header + drop record + window.
        let mut slab_bytes = Vec::new();
        slab_bytes.extend_from_slice(b"LIM1");
        slab_bytes.extend_from_slice(&1u16.to_le_bytes()); // format version
                                                           // SlabId: ordinal 0, content hash = drop_id (any 32 bytes).
        slab_bytes.extend_from_slice(&[0u8; 8]); // ordinal u64 LE = 0
        slab_bytes.extend_from_slice(&drop_id);
        let total_len: u64 = 56 + 50 + compressed.len() as u64; // header + drop record + window
        slab_bytes.extend_from_slice(&total_len.to_le_bytes());
        slab_bytes.push(0x00); // ec_descriptor
        slab_bytes.push(0x00); // crypto_hint
                               // Drop record (49 bytes): drop_id + plaintext_len(u32) +
                               // codec(u8) + aead(u8) + ec(u8) + swi(u8) + offset(u32) +
                               // len(u32) + dict_id(u8).
        slab_bytes.extend_from_slice(&drop_id);
        slab_bytes.extend_from_slice(&(plaintext.len() as u32).to_le_bytes());
        slab_bytes.push(crate::codec::CODEC_LZ4);
        slab_bytes.push(0x00);
        slab_bytes.push(0x00);
        slab_bytes.push(0x00);
        slab_bytes.extend_from_slice(&0u32.to_le_bytes()); // offset
        slab_bytes.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        slab_bytes.push(crate::drop_record::NO_DICT);
        slab_bytes.push(0x00); // flags
                               // Window.
        slab_bytes.extend_from_slice(&compressed);

        let store = SlabStore::from_bytes(vec![slab_bytes]).expect("slab parses");
        let cached = CachedSlabStore::with_default_capacity(store);

        // First call: cache miss.
        let pt1 = cached
            .plaintext_for(&drop_id)
            .expect("drop exists")
            .expect("decompress ok");
        assert_eq!(pt1, plaintext);
        assert_eq!(cached.cache_len(), 1, "first call should populate cache");

        // Second call: cache hit, same plaintext.
        let pt2 = cached
            .plaintext_for(&drop_id)
            .expect("drop exists")
            .expect("decompress ok");
        assert_eq!(pt2, plaintext);
        assert_eq!(cached.cache_len(), 1, "second call should hit, not insert");
    }
}
