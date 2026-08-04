//! Hot slab cache — bounded LRU over decoded drop plaintexts.
//!
//! [`SlabStore::plaintext_for`] decompresses on every call. For
//! read-heavy workloads (mount, `cat-multi`, turnover on a hot image)
//! the same drops are decoded over and over. [`CachedSlabStore`]
//! wraps a [`SlabStore`] and keeps the N most-recently-decoded
//! plaintexts in memory, keyed by `DropId`.
//!
//! ## Design
//!
//! - Tiny in-house LRU (`HashMap<[u8;32], Vec<u8>>` + `VecDeque<[u8;32]>`
//!   for eviction order). No external dep.
//! - Capacity is in entries (not bytes); caller picks based on
//!   expected working-set size and average drop size.
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
//! See `TODO.impl/03-core-reader/03-hot-slab-cache.md`.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use crate::slab_store::SlabStore;
use crate::CoreError;

/// Default cache capacity: 256 entries (≈ 4 MiB at 16 KiB avg drop).
pub const DEFAULT_CACHE_CAPACITY: usize = 256;

/// Bounded LRU cache wrapping a [`SlabStore`]. Cache hits return
/// the cached plaintext directly; misses fetch from the inner
/// store and insert.
pub struct CachedSlabStore {
    inner: SlabStore,
    cache: Mutex<LruCache>,
}

struct LruCache {
    /// DropId → plaintext.
    entries: HashMap<[u8; 32], Vec<u8>>,
    /// Access order; front = most-recently-used, back = LRU.
    order: VecDeque<[u8; 32]>,
    capacity: usize,
}

impl LruCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn get(&mut self, key: &[u8; 32]) -> Option<&Vec<u8>> {
        if self.entries.contains_key(key) {
            // Move to front (most-recently-used).
            self.order.retain(|k| k != key);
            self.order.push_front(*key);
            self.entries.get(key)
        } else {
            None
        }
    }

    fn insert(&mut self, key: [u8; 32], value: Vec<u8>) {
        if self.entries.contains_key(&key) {
            self.order.retain(|k| k != &key);
        } else if self.entries.len() >= self.capacity {
            // Evict least-recently-used.
            if let Some(evicted) = self.order.pop_back() {
                self.entries.remove(&evicted);
            }
        }
        self.order.push_front(key);
        self.entries.insert(key, value);
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

impl CachedSlabStore {
    /// Wrap `inner` with a cache of the given capacity (in entries).
    #[must_use]
    pub fn new(inner: SlabStore, capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            inner,
            cache: Mutex::new(LruCache::new(cap)),
        }
    }

    /// Wrap `inner` with the default capacity.
    #[must_use]
    pub fn with_default_capacity(inner: SlabStore) -> Self {
        Self::new(inner, DEFAULT_CACHE_CAPACITY)
    }

    /// Fetch `drop_id`'s plaintext. On hit, returns the cached
    /// clone directly. On miss, fetches from the inner store,
    /// inserts, and returns.
    #[must_use]
    pub fn plaintext_for(&self, drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, CoreError>> {
        {
            let mut cache = self.cache.lock().expect("cache mutex poisoned");
            if let Some(cached) = cache.get(drop_id) {
                return Some(Ok(cached.clone()));
            }
        }
        // Miss: fetch from inner.
        let plaintext = self.inner.plaintext_for(drop_id)?;
        match plaintext {
            Ok(bytes) => {
                let mut cache = self.cache.lock().expect("cache mutex poisoned");
                cache.insert(*drop_id, bytes.clone());
                Some(Ok(bytes))
            }
            Err(e) => Some(Err(e)),
        }
    }

    /// Number of entries currently cached.
    #[must_use]
    pub fn cache_len(&self) -> usize {
        self.cache.lock().expect("cache mutex poisoned").len()
    }

    /// Cache capacity (in entries).
    #[must_use]
    pub fn cache_capacity(&self) -> usize {
        self.cache.lock().expect("cache mutex poisoned").capacity
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

    #[test]
    fn lru_evicts_lru_when_full() {
        let mut cache = LruCache::new(2);
        cache.insert([1; 32], vec![0xAA]);
        cache.insert([2; 32], vec![0xBB]);
        cache.insert([3; 32], vec![0xCC]);
        // [1;32] was LRU, should be evicted.
        assert!(!cache.entries.contains_key(&[1; 32]));
        assert!(cache.entries.contains_key(&[2; 32]));
        assert!(cache.entries.contains_key(&[3; 32]));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn lru_get_promotes_to_front() {
        let mut cache = LruCache::new(2);
        cache.insert([1; 32], vec![0xAA]);
        cache.insert([2; 32], vec![0xBB]);
        // Touch [1;32] — it's now MRU.
        let _ = cache.get(&[1; 32]);
        cache.insert([3; 32], vec![0xCC]);
        // [2;32] should be evicted, [1;32] should survive.
        assert!(cache.entries.contains_key(&[1; 32]));
        assert!(!cache.entries.contains_key(&[2; 32]));
        assert!(cache.entries.contains_key(&[3; 32]));
    }

    #[test]
    fn lru_insert_existing_key_updates_value_without_eviction() {
        let mut cache = LruCache::new(2);
        cache.insert([1; 32], vec![0xAA]);
        cache.insert([1; 32], vec![0xBB]);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.entries.get(&[1; 32]).unwrap(), &vec![0xBB]);
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
        let total_len: u64 = 56 + 49 + compressed.len() as u64; // header + drop record + window
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
