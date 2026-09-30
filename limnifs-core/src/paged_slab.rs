//! Paged slab source — slabs over positioned (remote) byte sources.
//!
//! The reader-side half of lazy image mounting (tebako spec 39): a
//! slab whose bytes are NOT resident — they are fetched in pages
//! through a caller-supplied [`PositionedReader`] (an HTTP Range
//! transport, a file pread, a test mock). The record table (slab
//! header + drop records) is parsed EAGERLY at open over the
//! positioned reader — the solid window is never fetched for the
//! parse; drop payload bytes are fetched on demand at read time.
//!
//! Reader-side only: the wire format is unchanged, the writer is
//! unchanged, and [`crate::slab_store::SlabStore`] (`Memory`/`Mapped`)
//! is untouched. The byte-slice reader ([`crate::slab_reader`]) and
//! this module share the drop readability gates and the decode path
//! (`check_drop_readable` / `decode_drop_bytes`) so the two readers
//! answer IDENTICALLY over the same bytes — the parity property the
//! tests pin.
//!
//! The fetch quantum is one page ([`PAGED_PAGE_SIZE`]); consecutive
//! missing pages coalesce into a single underlying `read_at` call, so
//! a whole-drop read costs one request against a range-capable
//! transport while a record-table walk costs one for any slab whose
//! table fits in a page.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::cursor::ManifestCursor;
use crate::drop_record::{parse_drop_record, DropRecord, DROP_RECORD_LEN};
use crate::error::CoreError;
use crate::slab::{parse_slab_header, SlabHeader, SLAB_HEADER_LEN};
use crate::slab_reader::{check_drop_readable, decode_drop_bytes};

/// The page size of the paged source: the fetch quantum and the cache
/// entry size. 256 KiB covers a typical slab's whole record table in
/// one fetch (a 50-byte record walk starts at byte 56) and keeps a
/// 16-page cache at 4 MiB per slab. Consecutive missing pages coalesce
/// into one underlying call, so larger reads are not one-request-per-
/// page against a range transport.
pub const PAGED_PAGE_SIZE: u64 = 256 * 1024;

/// Default number of pages cached per slab ([`PagedSlab::open`]).
/// 16 pages × [`PAGED_PAGE_SIZE`] = 4 MiB — page 0 (the record table)
/// survives alongside sequential drop reads in practice.
pub const PAGED_CACHE_PAGES: usize = 16;

/// Sentinel for "object length not yet known" (before the slab header
/// is parsed / EOF is observed).
const UNKNOWN_LEN: u64 = u64::MAX;

/// A positioned byte source — the abstraction a range transport
/// implements for this module.
///
/// `pread` semantics: `read_at(offset, len)` returns UP TO `len`
/// bytes; a short return means the object ended (EOF), an empty return
/// means `offset` is at or past the end. This is exactly what an HTTP
/// 206 answer to a Range request that overruns the object delivers,
/// and what `pread(2)` delivers. Implementations MUST be deterministic
/// (the same range always yields the same bytes — content addressing
/// depends on it) and MUST NOT return MORE than `len` bytes.
///
/// A blanket impl covers `Fn(u64, usize) -> Result<Vec<u8>, CoreError>`
/// closures, so callers can adapt any fetch surface without a newtype.
pub trait PositionedReader: Send + Sync {
    /// Read up to `len` bytes at `offset` (short only at EOF).
    ///
    /// # Errors
    ///
    /// Returns [`CoreError`] on transport failure. Convention (as in
    /// [`crate::slab_store::SlabStore`]): transport failures ride
    /// [`CoreError::Corrupt`] with a `reason` naming the source.
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, CoreError>;
}

impl<F> PositionedReader for F
where
    F: Fn(u64, usize) -> Result<Vec<u8>, CoreError> + Send + Sync,
{
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, CoreError> {
        self(offset, len)
    }
}

/// A tiny LRU over fetched pages. Hand-rolled (no dependency): the
/// capacity is small (default [`PAGED_CACHE_PAGES`]), so a linear scan
/// is both simplest and fastest.
struct PageCache {
    /// `(page_index, bytes)`; the back is the most recently used.
    entries: Vec<(u64, Arc<[u8]>)>,
    capacity: usize,
}

impl PageCache {
    const fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            capacity,
        }
    }

    fn get(&mut self, index: u64) -> Option<Arc<[u8]>> {
        let pos = self.entries.iter().position(|(i, _)| *i == index)?;
        let (i, bytes) = self.entries.remove(pos);
        let hit = Arc::clone(&bytes);
        self.entries.push((i, bytes));
        Some(hit)
    }

    fn insert(&mut self, index: u64, bytes: Arc<[u8]>) {
        if let Some(pos) = self.entries.iter().position(|(i, _)| *i == index) {
            self.entries.remove(pos);
        }
        self.entries.push((index, bytes));
        while self.entries.len() > self.capacity {
            self.entries.remove(0);
        }
    }
}

/// A positioned reader with a page cache in front of it.
///
/// Every underlying fetch is page-aligned; consecutive missing pages
/// coalesce into one underlying call. The object length starts
/// UNKNOWN (the slab header has not been parsed yet); a short
/// underlying return teaches it (EOF), and [`PagedBytes::set_len`]
/// pins it authoritatively once the header is parsed. Reads clamp at
/// a known length (`pread` semantics); reads while the length is
/// unknown simply come back short if the object ends first.
pub struct PagedBytes {
    reader: Arc<dyn PositionedReader>,
    len: AtomicU64,
    cache: Mutex<PageCache>,
}

impl PagedBytes {
    /// Wrap `reader` with the default cache ([`PAGED_CACHE_PAGES`]).
    #[must_use]
    pub fn new(reader: Arc<dyn PositionedReader>) -> Self {
        Self::with_cache_pages(reader, PAGED_CACHE_PAGES)
    }

    /// Wrap `reader` caching at most `cache_pages` pages (`0` disables
    /// caching — every read refetches).
    #[must_use]
    pub fn with_cache_pages(reader: Arc<dyn PositionedReader>, cache_pages: usize) -> Self {
        Self {
            reader,
            len: AtomicU64::new(UNKNOWN_LEN),
            cache: Mutex::new(PageCache::with_capacity(cache_pages)),
        }
    }

    /// The object length, once known (header parsed or EOF observed).
    #[must_use]
    pub fn len(&self) -> Option<u64> {
        let v = self.len.load(Ordering::Acquire);
        if v == UNKNOWN_LEN {
            None
        } else {
            Some(v)
        }
    }

    /// Pin the object length (the slab header's `total_length`).
    /// Authoritative: later EOF observations never shrink it.
    pub fn set_len(&self, len: u64) {
        self.len.store(len, Ordering::Release);
    }

    /// Record an observed EOF (a short underlying return). Only the
    /// first observation counts, and never against a pinned length.
    fn learn_len(&self, len: u64) {
        let _ = self
            .len
            .compare_exchange(UNKNOWN_LEN, len, Ordering::AcqRel, Ordering::Acquire);
    }

    /// `pread`-style read through the page cache: up to `len` bytes at
    /// `offset`, short only when the object ends first, empty at or
    /// past the end.
    ///
    /// # Errors
    ///
    /// - [`CoreError::Corrupt`] if the range arithmetic overflows, or
    ///   if the underlying reader violates the contract (returns more
    ///   than requested).
    /// - Propagates the underlying reader's transport errors.
    pub fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, CoreError> {
        let want_end = offset
            .checked_add(len as u64)
            .ok_or_else(|| CoreError::Corrupt {
                reason: format!("paged read offset {offset} + length {len} overflows u64"),
            })?;
        if want_end == offset {
            return Ok(Vec::new());
        }
        // Clamp at the known end, pread-style.
        let end = match self.len() {
            Some(known) => want_end.min(known),
            None => want_end,
        };
        if end <= offset {
            return Ok(Vec::new());
        }
        let first = offset / PAGED_PAGE_SIZE;
        let last = (end - 1) / PAGED_PAGE_SIZE;

        // Snapshot the cache for every page in the range.
        let mut pages: Vec<Option<Arc<[u8]>>> = Vec::with_capacity((last - first + 1) as usize);
        {
            let mut cache = self
                .cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for index in first..=last {
                pages.push(cache.get(index));
            }
        }

        // Fetch the missing pages, one underlying call per CONSECUTIVE
        // run (a whole-drop read is one request on a range transport).
        let mut i = 0;
        while i < pages.len() {
            if pages[i].is_some() {
                i += 1;
                continue;
            }
            let run_start = i;
            while i < pages.len() && pages[i].is_none() {
                i += 1;
            }
            let fetch_offset = (first + run_start as u64) * PAGED_PAGE_SIZE;
            let run_end = (first + i as u64) * PAGED_PAGE_SIZE;
            let fetch_end = match self.len() {
                Some(known) => run_end.min(known),
                None => run_end,
            };
            let expected =
                usize::try_from(fetch_end - fetch_offset).map_err(|_| CoreError::Corrupt {
                    reason: format!("paged fetch of {fetch_end} - {fetch_offset} exceeds usize"),
                })?;
            let bytes = self.reader.read_at(fetch_offset, expected)?;
            if bytes.len() > expected {
                return Err(CoreError::Corrupt {
                    reason: format!(
                        "paged source over-delivered: {} bytes for a {expected}-byte request at offset {fetch_offset}",
                        bytes.len()
                    ),
                });
            }
            if bytes.len() < expected {
                self.learn_len(fetch_offset + bytes.len() as u64);
            }
            let mut cache = self
                .cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (j, chunk) in bytes.chunks(PAGED_PAGE_SIZE as usize).enumerate() {
                let index = first + (run_start + j) as u64;
                let page: Arc<[u8]> = Arc::from(chunk);
                cache.insert(index, Arc::clone(&page));
                pages[run_start + j] = Some(page);
            }
        }

        // Assemble, stopping short at the object's end (a missing or
        // short page mid-range can only be EOF — every in-range page
        // was just fetched).
        let mut out = Vec::with_capacity((end - offset) as usize);
        for (slot, page) in pages.iter().enumerate() {
            let Some(page) = page else { break };
            let page_start = (first + slot as u64) * PAGED_PAGE_SIZE;
            let from = offset.max(page_start) - page_start;
            let to = end.min(page_start + PAGED_PAGE_SIZE) - page_start;
            let available = (page.len() as u64).min(to);
            if available <= from {
                break;
            }
            out.extend_from_slice(&page[from as usize..available as usize]);
            if available < to {
                break;
            }
        }
        Ok(out)
    }
}

impl PositionedReader for PagedBytes {
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, CoreError> {
        PagedBytes::read_at(self, offset, len)
    }
}

/// A slab's eagerly-parsed prefix: header + every drop record + the
/// solid-window boundary. Everything a caller needs to locate drop
/// bytes; none of the payload.
#[derive(Debug, Clone)]
pub struct SlabRecordTable {
    /// The slab header (`total_length` is the slab's byte extent).
    pub header: SlabHeader,
    /// Every drop record, in declaration order.
    pub drop_records: Vec<DropRecord>,
    /// Byte offset where the solid window begins (immediately after
    /// the last drop record).
    pub solid_window_start: u64,
}

/// Parse a slab's record table over a positioned reader: the header,
/// then the drop-record walk, stopping at the derived solid-window
/// boundary — the same derivation [`crate::slab_reader::parse_slab`]
/// performs over a resident buffer, but fetching only header + record
/// bytes, never the solid window.
///
/// Unlike [`crate::slab_reader::parse_slab`], there is no resident
/// buffer to compare `total_length` against: the header's
/// `total_length` IS the slab extent here, and the object's integrity
/// is the source's contract (a range transport pins it by digest).
///
/// # Errors
///
/// - Inherits errors from [`parse_slab_header`] and
///   [`parse_drop_record`], and propagates the reader's transport
///   errors.
/// - [`CoreError::TooShort`] if the object ends inside the header or
///   a record.
/// - [`CoreError::Corrupt`] if the record walk cannot derive the
///   solid-window boundary consistently.
pub fn parse_slab_record_table(
    reader: &dyn PositionedReader,
) -> Result<SlabRecordTable, CoreError> {
    let header_bytes = reader.read_at(0, SLAB_HEADER_LEN)?;
    if header_bytes.len() != SLAB_HEADER_LEN {
        return Err(CoreError::TooShort {
            have: header_bytes.len(),
            need: SLAB_HEADER_LEN,
        });
    }
    let mut cursor = ManifestCursor::new(&header_bytes);
    let header = parse_slab_header(&mut cursor)?;

    let mut drop_records: Vec<DropRecord> = Vec::new();
    let mut window_len_sum: u64 = 0;
    let mut pos = SLAB_HEADER_LEN as u64;
    loop {
        let remaining_after_cursor =
            header
                .total_length
                .checked_sub(pos)
                .ok_or_else(|| CoreError::Corrupt {
                    reason: format!(
                        "slab cursor position {pos} past total_length {}",
                        header.total_length
                    ),
                })?;
        if remaining_after_cursor == window_len_sum {
            break;
        }
        if remaining_after_cursor < window_len_sum {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "slab drop records overran solid window: cursor_pos={pos}, window_sum={window_len_sum}, total_length={}",
                    header.total_length
                ),
            });
        }
        let trailing = remaining_after_cursor - window_len_sum;
        if trailing < DROP_RECORD_LEN as u64 {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "slab has {trailing} trailing bytes that are neither a full drop record nor accounted for by the solid window"
                ),
            });
        }
        let record_bytes = reader.read_at(pos, DROP_RECORD_LEN)?;
        if record_bytes.len() != DROP_RECORD_LEN {
            return Err(CoreError::TooShort {
                have: record_bytes.len(),
                need: DROP_RECORD_LEN,
            });
        }
        let mut record_cursor = ManifestCursor::new(&record_bytes);
        let record = parse_drop_record(&mut record_cursor, &header)?;
        window_len_sum = window_len_sum
            .checked_add(u64::from(record.len_in_window))
            .ok_or_else(|| CoreError::Corrupt {
                reason: format!(
                    "slab drop len_in_window sum overflow at record {}",
                    drop_records.len()
                ),
            })?;
        drop_records.push(record);
        pos += DROP_RECORD_LEN as u64;
    }

    Ok(SlabRecordTable {
        header,
        drop_records,
        solid_window_start: pos,
    })
}

/// One slab over a paged source: the record table parsed eagerly at
/// open, drop payload bytes fetched on demand through the page cache.
/// The positioned-reader analogue of
/// [`crate::slab_reader::SlabView`]; both decode through the same
/// shared gates, so they answer identically over the same bytes.
pub struct PagedSlab {
    paged: PagedBytes,
    table: SlabRecordTable,
}

impl PagedSlab {
    /// Open a slab over `reader` with the default page cache,
    /// parsing the record table eagerly (page-cached: a slab whose
    /// table fits in page 0 costs one underlying fetch).
    ///
    /// # Errors
    ///
    /// Inherits errors from [`parse_slab_record_table`].
    pub fn open(reader: Arc<dyn PositionedReader>) -> Result<Self, CoreError> {
        Self::open_with_cache_pages(reader, PAGED_CACHE_PAGES)
    }

    /// [`PagedSlab::open`] with an explicit page-cache capacity
    /// (`0` disables caching).
    ///
    /// # Errors
    ///
    /// Inherits errors from [`parse_slab_record_table`].
    pub fn open_with_cache_pages(
        reader: Arc<dyn PositionedReader>,
        cache_pages: usize,
    ) -> Result<Self, CoreError> {
        let paged = PagedBytes::with_cache_pages(reader, cache_pages);
        let table = parse_slab_record_table(&paged)?;
        paged.set_len(table.header.total_length);
        Ok(Self { paged, table })
    }

    /// The slab header.
    #[must_use]
    pub const fn header(&self) -> SlabHeader {
        self.table.header
    }

    /// The slab's byte extent (the header's `total_length`).
    #[must_use]
    pub const fn byte_len(&self) -> u64 {
        self.table.header.total_length
    }

    /// All drop records in this slab, in declaration order.
    #[must_use]
    pub fn drop_records(&self) -> &[DropRecord] {
        &self.table.drop_records
    }

    /// Byte offset where the solid window begins.
    #[must_use]
    pub const fn solid_window_offset(&self) -> u64 {
        self.table.solid_window_start
    }

    /// Find a drop record by its `DropId`. Linear scan.
    #[must_use]
    pub fn find_record(&self, drop_id: &[u8; 32]) -> Option<&DropRecord> {
        self.table
            .drop_records
            .iter()
            .find(|r| r.drop_id.as_bytes() == drop_id)
    }

    /// `pread`-style slab read through the page cache.
    ///
    /// # Errors
    ///
    /// Inherits errors from [`PagedBytes::read_at`].
    pub fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, CoreError> {
        self.paged.read_at(offset, len)
    }

    /// Fetch and decode the plaintext of `drop_id`, or `None` if no
    /// drop in this slab carries that id. The on-demand counterpart of
    /// [`crate::slab_reader::SlabView::plaintext_for`].
    #[must_use]
    pub fn plaintext_for(&self, drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, CoreError>> {
        self.plaintext_for_with_dict_lookup(drop_id, &|_| None)
    }

    /// [`PagedSlab::plaintext_for`] with a callback resolving
    /// `dict_id` → dictionary bytes, mirroring
    /// [`crate::slab_reader::SlabView::plaintext_for_with_dict_lookup`].
    #[must_use]
    pub fn plaintext_for_with_dict_lookup(
        &self,
        drop_id: &[u8; 32],
        dict_lookup: &dyn Fn(u8) -> Option<Vec<u8>>,
    ) -> Option<Result<Vec<u8>, CoreError>> {
        let record = self.find_record(drop_id)?;
        if let Err(e) = check_drop_readable(record) {
            return Some(Err(e));
        }
        let start = self
            .table
            .solid_window_start
            .checked_add(u64::from(record.offset_in_window))?;
        let end = start.checked_add(u64::from(record.len_in_window))?;
        if end > self.byte_len() {
            return Some(Err(CoreError::Corrupt {
                reason: format!(
                    "drop range [{start}..{end}] extends past slab length {}",
                    self.byte_len()
                ),
            }));
        }
        let len = usize::try_from(record.len_in_window).ok()?;
        let raw = match self.paged.read_at(start, len) {
            Ok(raw) => raw,
            Err(e) => return Some(Err(e)),
        };
        if raw.len() != len {
            return Some(Err(CoreError::Corrupt {
                reason: format!(
                    "drop bytes truncated: {} of {len} bytes at slab offset {start}",
                    raw.len()
                ),
            }));
        }
        Some(decode_drop_bytes(record, &raw, dict_lookup))
    }
}

/// Every slab of one image over paged sources, with a
/// `DropId` → slab-ordinal index for O(1) lookup — the
/// positioned-reader counterpart of [`crate::slab_store::SlabStore`].
/// Implements the [`crate::slab_source::SlabSource`] trait, so
/// consumers written against the store abstraction take it unchanged.
#[derive(Default)]
pub struct PagedSlabSet {
    slabs: Vec<PagedSlab>,
    drop_index: HashMap<[u8; 32], usize>,
    dictionaries: HashMap<u8, Vec<u8>>,
}

impl PagedSlabSet {
    /// Open every slab, parsing each record table eagerly and
    /// building the drop index. An empty `readers` yields an empty
    /// set (the [`crate::slab_store::SlabStore::load`] convention).
    ///
    /// # Errors
    ///
    /// Inherits errors from [`PagedSlab::open`], with the slab ordinal
    /// prefixed to `Corrupt` reasons.
    pub fn open(readers: Vec<Arc<dyn PositionedReader>>) -> Result<Self, CoreError> {
        let mut slabs = Vec::with_capacity(readers.len());
        let mut drop_index: HashMap<[u8; 32], usize> = HashMap::new();
        for (ordinal, reader) in readers.into_iter().enumerate() {
            let slab = PagedSlab::open(reader).map_err(|e| match e {
                CoreError::Corrupt { reason } => CoreError::Corrupt {
                    reason: format!("slab ordinal {ordinal}: {reason}"),
                },
                other => other,
            })?;
            for record in slab.drop_records() {
                drop_index.insert(*record.drop_id.as_bytes(), ordinal);
            }
            slabs.push(slab);
        }
        Ok(Self {
            slabs,
            drop_index,
            dictionaries: HashMap::new(),
        })
    }

    /// One slab by ordinal.
    #[must_use]
    pub fn slab(&self, ordinal: usize) -> Option<&PagedSlab> {
        self.slabs.get(ordinal)
    }

    /// Number of slabs in the set.
    #[must_use]
    pub fn slab_count(&self) -> usize {
        self.slabs.len()
    }

    /// Number of unique drops indexed across all slabs.
    #[must_use]
    pub fn drop_count(&self) -> usize {
        self.drop_index.len()
    }

    /// Returns true if `drop_id` is present in any slab.
    #[must_use]
    pub fn contains(&self, drop_id: &[u8; 32]) -> bool {
        self.drop_index.contains_key(drop_id)
    }

    /// Set the dictionary table parsed from the manifest's
    /// `dictionary_section` (the
    /// [`crate::slab_store::SlabStore::set_dictionaries`] contract).
    pub fn set_dictionaries(&mut self, dictionaries: HashMap<u8, Vec<u8>>) {
        self.dictionaries = dictionaries;
    }

    /// Number of registered dictionaries.
    #[must_use]
    pub fn dictionary_count(&self) -> usize {
        self.dictionaries.len()
    }

    /// Fetch and decode the plaintext of `drop_id` from whichever
    /// slab holds it. Mirrors
    /// [`crate::slab_store::SlabStore::plaintext_for`].
    #[must_use]
    pub fn plaintext_for(&self, drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, CoreError>> {
        let ordinal = *self.drop_index.get(drop_id)?;
        let slab = self.slabs.get(ordinal)?;
        slab.plaintext_for_with_dict_lookup(drop_id, &|id| self.dictionaries.get(&id).cloned())
    }
}

impl crate::slab_source::SlabSource for PagedSlabSet {
    fn plaintext_for(&self, drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, CoreError>> {
        PagedSlabSet::plaintext_for(self, drop_id)
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
    use crate::drop_record::NO_DICT;
    use crate::slab_reader::parse_slab;
    use std::sync::atomic::AtomicUsize;

    /// Build a store-codec slab (the same encoding
    /// `slab_reader::tests::make_slab` produces).
    fn make_slab(drops: &[(&[u8; 32], &[u8])]) -> Vec<u8> {
        let mut drop_records = Vec::new();
        let mut solid_window = Vec::new();
        for (id, plaintext) in drops {
            let plaintext_len = u32::try_from(plaintext.len()).unwrap();
            let offset_in_window = u32::try_from(solid_window.len()).unwrap();
            drop_records.extend_from_slice(*id);
            drop_records.extend_from_slice(&plaintext_len.to_le_bytes());
            drop_records.extend_from_slice(&[0x00, 0x00, 0x00]); // store, plaintext, no EC
            drop_records.push(0x00); // solid_window_index
            drop_records.extend_from_slice(&offset_in_window.to_le_bytes());
            drop_records.extend_from_slice(&plaintext_len.to_le_bytes());
            drop_records.push(NO_DICT);
            drop_records.push(0x00); // flags
            solid_window.extend_from_slice(plaintext);
        }
        let slab_content = [&drop_records[..], &solid_window[..]].concat();
        let total_length = (SLAB_HEADER_LEN + slab_content.len()) as u64;
        let mut bytes = Vec::with_capacity(total_length as usize);
        bytes.extend_from_slice(b"LIM1");
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes()); // ordinal
        bytes.extend_from_slice(&[0u8; 32]); // hash
        bytes.extend_from_slice(&total_length.to_le_bytes());
        bytes.push(0x00); // ec_descriptor
        bytes.push(0x00); // crypto_hint
        bytes.extend_from_slice(&slab_content);
        bytes
    }

    /// A `pread`-semantics mock over owned bytes, recording every call.
    struct MockBytes {
        bytes: Vec<u8>,
        calls: Mutex<Vec<(u64, usize)>>,
        call_seq: AtomicUsize,
        fail_on_call: AtomicUsize,
    }

    impl MockBytes {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                calls: Mutex::new(Vec::new()),
                call_seq: AtomicUsize::new(0),
                fail_on_call: AtomicUsize::new(usize::MAX),
            }
        }

        fn call_count(&self) -> usize {
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
        }

        fn calls(&self) -> Vec<(u64, usize)> {
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }

        fn fail_on(&self, n: usize) {
            self.fail_on_call.store(n, Ordering::SeqCst);
        }
    }

    impl PositionedReader for MockBytes {
        fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, CoreError> {
            let seq = self.call_seq.fetch_add(1, Ordering::SeqCst) + 1;
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((offset, len));
            if seq == self.fail_on_call.load(Ordering::SeqCst) {
                return Err(CoreError::Corrupt {
                    reason: "injected fetch failure".into(),
                });
            }
            let start = usize::try_from(offset).unwrap_or(self.bytes.len());
            if start >= self.bytes.len() {
                return Ok(Vec::new());
            }
            let take = len.min(self.bytes.len() - start);
            Ok(self.bytes[start..start + take].to_vec())
        }
    }

    /// Deterministic PRNG for the parity property (no dev-dependency).
    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    // -------------------- record-table parse --------------------

    #[test]
    fn record_table_parse_matches_slice_parse() {
        let id1 = [0x11; 32];
        let id2 = [0x22; 32];
        let id3 = [0x33; 32];
        let bytes = make_slab(&[
            (&id1, b"first drop plaintext"),
            (&id2, b"second"),
            (&id3, b"third drop is longer than the others combined"),
        ]);
        let mock = MockBytes::new(bytes.clone());
        let table = parse_slab_record_table(&mock).expect("table parses");
        let view = parse_slab(&bytes).expect("slice parse");
        assert_eq!(table.header, view.header());
        assert_eq!(table.drop_records, view.drop_records());
        assert_eq!(table.solid_window_start, view.solid_window_offset() as u64);
        // The parse fetched only header + record bytes, never the window.
        let fetched: u64 = mock
            .calls()
            .iter()
            .map(|(offset, len)| offset + *len as u64)
            .max()
            .unwrap_or(0);
        assert!(
            fetched <= table.solid_window_start,
            "fetched up to {fetched}, window starts {}",
            table.solid_window_start
        );
    }

    #[test]
    fn record_table_parse_handles_empty_slab() {
        let bytes = make_slab(&[]);
        let mock = MockBytes::new(bytes);
        let table = parse_slab_record_table(&mock).expect("empty table parses");
        assert!(table.drop_records.is_empty());
        assert_eq!(table.solid_window_start, SLAB_HEADER_LEN as u64);
    }

    #[test]
    fn record_table_parse_names_trailing_junk() {
        // One drop, but total_length claims one extra byte beyond what
        // records + window account for.
        let id = [0xAA; 32];
        let mut bytes = make_slab(&[(&id, b"data")]);
        let total = u64::from_le_bytes(bytes[46..54].try_into().unwrap()) + 1;
        bytes[46..54].copy_from_slice(&total.to_le_bytes());
        bytes.push(0x00);
        let mock = MockBytes::new(bytes);
        match parse_slab_record_table(&mock) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("trailing"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn record_table_parse_names_a_short_object() {
        // The object ends inside the second record.
        let id1 = [0x11; 32];
        let id2 = [0x22; 32];
        let mut bytes = make_slab(&[(&id1, b"aaaa"), (&id2, b"bbbb")]);
        bytes.truncate(SLAB_HEADER_LEN + DROP_RECORD_LEN + 10);
        let mock = MockBytes::new(bytes);
        match parse_slab_record_table(&mock) {
            Err(CoreError::TooShort { .. }) => {}
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    // -------------------- paged reads and the cache --------------------

    fn big_slab() -> (Vec<u8>, Vec<([u8; 32], Vec<u8>)>) {
        // Three drops, the last spanning several pages.
        let id1 = [0x11; 32];
        let id2 = [0x22; 32];
        let id3 = [0x33; 32];
        let p1 = vec![0xAB; 4096];
        let p2 = vec![0xCD; 1024];
        let p3: Vec<u8> = (0..(PAGED_PAGE_SIZE as usize * 3 + 12345))
            .map(|i| (i % 251) as u8)
            .collect();
        let slab = make_slab(&[(&id1, &p1), (&id2, &p2), (&id3, &p3)]);
        (slab, vec![(id1, p1), (id2, p2), (id3, p3)])
    }

    #[test]
    fn paged_open_fetches_the_table_in_one_call() {
        let (slab, _) = big_slab();
        let mock = MockBytes::new(slab);
        let paged = PagedSlab::open(Arc::new(mock)).expect("open");
        assert_eq!(paged.drop_records().len(), 3);
        // Header + records all sit in page 0: one underlying fetch.
        // (The MockBytes is behind the Arc; re-check via a fresh count.)
    }

    #[test]
    fn paged_open_costs_one_fetch_for_a_small_slab() {
        let id = [0xAA; 32];
        let slab = make_slab(&[(&id, b"tiny")]);
        let mock = Arc::new(MockBytes::new(slab));
        let paged = PagedSlab::open(Arc::clone(&mock) as Arc<dyn PositionedReader>).expect("open");
        assert_eq!(mock.call_count(), 1, "calls: {:?}", mock.calls());
        assert_eq!(mock.calls()[0].0, 0);
        assert_eq!(paged.drop_records().len(), 1);
    }

    #[test]
    fn read_at_matches_the_resident_slice() {
        let (slab, _) = big_slab();
        let paged = PagedSlab::open(Arc::new(MockBytes::new(slab.clone()))).expect("open");
        // Windows: inside one page, across the boundary, the final
        // partial page, a zero-length read, and a clamped pread tail.
        let total = slab.len() as u64;
        for (offset, len) in [
            (0u64, 100usize),
            (PAGED_PAGE_SIZE - 7, 14),
            (PAGED_PAGE_SIZE, PAGED_PAGE_SIZE as usize),
            (total - 10, 10),
            (total - 5, 500), // clamps at EOF, pread-style
            (total, 8),       // at EOF: empty
            (17, 0),          // zero length: empty
        ] {
            let got = paged.read_at(offset, len).expect("read");
            let want_end = (offset + len as u64).min(total) as usize;
            let want = &slab[offset.min(total) as usize..want_end];
            assert_eq!(got, want, "window [{offset}..+{len}]");
        }
    }

    #[test]
    fn cache_serves_repeated_reads_without_refetch() {
        let (slab, _) = big_slab();
        let mock = Arc::new(MockBytes::new(slab));
        let paged = PagedSlab::open(Arc::clone(&mock) as Arc<dyn PositionedReader>).expect("open");
        let before = mock.call_count();
        let first = paged.read_at(PAGED_PAGE_SIZE + 100, 200).expect("first");
        let after_first = mock.call_count();
        assert_eq!(after_first, before + 1, "one page fetch");
        let second = paged.read_at(PAGED_PAGE_SIZE + 100, 200).expect("second");
        assert_eq!(mock.call_count(), after_first, "second read is cached");
        assert_eq!(first, second);
    }

    #[test]
    fn consecutive_missing_pages_coalesce_into_one_call() {
        let (slab, _) = big_slab();
        let mock = Arc::new(MockBytes::new(slab));
        let paged = PagedSlab::open(Arc::clone(&mock) as Arc<dyn PositionedReader>).expect("open");
        let before = mock.call_count();
        // Pages 1 and 2 sit fully inside the solid window: a two-page
        // read misses both and must coalesce into one underlying call.
        let got = paged
            .read_at(PAGED_PAGE_SIZE, (PAGED_PAGE_SIZE * 2) as usize)
            .expect("two-page read");
        assert_eq!(got.len(), (PAGED_PAGE_SIZE * 2) as usize);
        assert_eq!(
            mock.call_count(),
            before + 1,
            "one coalesced call: {:?}",
            &mock.calls()[before..]
        );
    }

    #[test]
    fn lru_evicts_the_least_recently_used_page() {
        let (slab, _) = big_slab();
        let mock = Arc::new(MockBytes::new(slab));
        let paged =
            PagedSlab::open_with_cache_pages(Arc::clone(&mock) as Arc<dyn PositionedReader>, 2)
                .expect("open");
        // Pages 1, 2, 3 live in the solid window; capacity is 2.
        let before = mock.call_count();
        paged.read_at(PAGED_PAGE_SIZE, 1).expect("page 1");
        paged.read_at(PAGED_PAGE_SIZE * 2, 1).expect("page 2");
        paged
            .read_at(PAGED_PAGE_SIZE * 3, 1)
            .expect("page 3 evicts 1");
        assert_eq!(mock.call_count(), before + 3);
        // Page 2 is still cached (page 1 was the LRU)…
        paged
            .read_at(PAGED_PAGE_SIZE * 2, 1)
            .expect("page 2 cached");
        assert_eq!(mock.call_count(), before + 3, "page 2 stayed resident");
        // …but page 1 was evicted and refetches.
        paged.read_at(PAGED_PAGE_SIZE, 1).expect("page 1 refetched");
        assert_eq!(mock.call_count(), before + 4);
    }

    // -------------------- drop plaintext parity --------------------

    #[test]
    fn plaintext_matches_the_slice_reader_drop_for_drop() {
        let (slab, drops) = big_slab();
        let view = parse_slab(&slab).expect("slice parse");
        let paged = PagedSlab::open(Arc::new(MockBytes::new(slab.clone()))).expect("open");
        for (id, plaintext) in &drops {
            let via_paged = paged.plaintext_for(id).expect("present").expect("decodes");
            let via_slice = view.plaintext_for(id).expect("present").expect("decodes");
            assert_eq!(&via_paged, plaintext);
            assert_eq!(via_paged, via_slice);
        }
        let missing = [0xEE; 32];
        assert!(paged.plaintext_for(&missing).is_none());
        assert!(view.plaintext_for(&missing).is_none());
    }

    #[test]
    fn readability_gates_match_the_slice_reader() {
        // A sealed slab (crypto_hint=1) may declare aead=1 records;
        // both readers must reject them with the same named error.
        let id = [0x77; 32];
        let mut drop_record = Vec::new();
        drop_record.extend_from_slice(&id);
        drop_record.extend_from_slice(&4u32.to_le_bytes()); // plaintext_len
        drop_record.extend_from_slice(&[0x00, 0x01, 0x00]); // store, AEAD, no EC
        drop_record.push(0x00);
        drop_record.extend_from_slice(&0u32.to_le_bytes());
        drop_record.extend_from_slice(&4u32.to_le_bytes());
        drop_record.push(NO_DICT);
        drop_record.push(0x00); // flags
        let content = [&drop_record[..], b"zzzz"].concat();
        let total_length = (SLAB_HEADER_LEN + content.len()) as u64;
        let mut slab = Vec::new();
        slab.extend_from_slice(b"LIM1");
        slab.extend_from_slice(&1u16.to_le_bytes());
        slab.extend_from_slice(&0u64.to_le_bytes());
        slab.extend_from_slice(&[0u8; 32]);
        slab.extend_from_slice(&total_length.to_le_bytes());
        slab.push(0x00); // ec_descriptor
        slab.push(0x01); // crypto_hint: sealed
        slab.extend_from_slice(&content);

        let view = parse_slab(&slab).expect("slice parse");
        let paged = PagedSlab::open(Arc::new(MockBytes::new(slab.clone()))).expect("open");
        let slice_err = view.plaintext_for(&id).expect("present").expect_err("err");
        let paged_err = paged.plaintext_for(&id).expect("present").expect_err("err");
        assert_eq!(slice_err, paged_err);
        assert!(matches!(slice_err, CoreError::UnsupportedFeature { .. }));
    }

    // -------------------- fault injection --------------------

    #[test]
    fn a_failing_table_fetch_is_a_named_error_not_a_panic() {
        let (slab, _) = big_slab();
        let mock = Arc::new(MockBytes::new(slab));
        mock.fail_on(1);
        match PagedSlab::open(mock as Arc<dyn PositionedReader>) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("injected fetch failure"), "got: {reason}");
            }
            Ok(_) => panic!("expected Corrupt, got Ok"),
            Err(other) => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn a_failing_drop_fetch_is_a_named_error_not_a_panic() {
        let (slab, drops) = big_slab();
        let mock = Arc::new(MockBytes::new(slab));
        let paged = PagedSlab::open(Arc::clone(&mock) as Arc<dyn PositionedReader>).expect("open");
        mock.fail_on(mock.call_count() + 1);
        // The third drop spans pages 1..3 — not fetched at open, so its
        // read goes to the (failing) underlying reader.
        let err = paged
            .plaintext_for(&drops[2].0)
            .expect("present")
            .expect_err("err");
        assert!(
            matches!(&err, CoreError::Corrupt { reason } if reason.contains("injected fetch failure")),
            "got: {err:?}"
        );
    }

    // -------------------- the parity property --------------------

    #[test]
    fn paged_and_slice_readers_answer_identically_over_random_slabs() {
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
        for iteration in 0..100u64 {
            // A random slab: 0..24 drops of 0..~600 KiB each.
            let drop_count = rng.below(25);
            let mut drops: Vec<([u8; 32], Vec<u8>)> = Vec::new();
            for d in 0..drop_count {
                let mut id = [0u8; 32];
                id[..8].copy_from_slice(&(d as u64).to_le_bytes());
                id[8..16].copy_from_slice(&iteration.to_le_bytes());
                let size = rng.below(600_000) as usize;
                let plaintext: Vec<u8> = (0..size).map(|_| (rng.next() % 251) as u8).collect();
                drops.push((id, plaintext));
            }
            let slab = make_slab(
                &drops
                    .iter()
                    .map(|(id, p)| (id, p.as_slice()))
                    .collect::<Vec<_>>(),
            );
            let total = slab.len() as u64;

            let view = parse_slab(&slab).expect("slice parse");
            let mock = Arc::new(MockBytes::new(slab.clone()));
            let paged =
                PagedSlab::open(Arc::clone(&mock) as Arc<dyn PositionedReader>).expect("open");

            // Table parity.
            assert_eq!(paged.header(), view.header(), "iteration {iteration}");
            assert_eq!(paged.drop_records(), view.drop_records());
            assert_eq!(
                paged.solid_window_offset(),
                view.solid_window_offset() as u64
            );

            // Random window reads: pread-parity with the resident slice.
            for _ in 0..64 {
                let offset = rng.below(total + 128);
                let len = rng.below(PAGED_PAGE_SIZE * 2 + 977) as usize;
                let got = paged.read_at(offset, len).expect("read");
                let clamped_start = offset.min(total) as usize;
                let clamped_end = (offset + len as u64).min(total) as usize;
                assert_eq!(
                    got,
                    &slab[clamped_start..clamped_end.max(clamped_start)],
                    "iteration {iteration} window [{offset}..+{len}]"
                );
            }

            // Drop-for-drop plaintext parity.
            for (id, _) in &drops {
                assert_eq!(
                    paged.plaintext_for(id),
                    view.plaintext_for(id),
                    "iteration {iteration}"
                );
            }
            // Missing ids agree too.
            for _ in 0..8 {
                let mut missing = [0u8; 32];
                missing[..8].copy_from_slice(&rng.next().to_le_bytes());
                missing[8] = 0xFF; // outside the generated id space
                assert_eq!(
                    paged.plaintext_for(&missing).is_none(),
                    view.plaintext_for(&missing).is_none()
                );
            }
        }
    }

    // -------------------- the set --------------------

    #[test]
    fn paged_slab_set_matches_the_store() {
        let id1 = [0x11; 32];
        let id2 = [0x22; 32];
        let id3 = [0x33; 32];
        let slab0 = make_slab(&[(&id1, b"first"), (&id2, b"second")]);
        let slab1 = make_slab(&[(&id3, b"third in slab one")]);

        let store = crate::slab_store::SlabStore::from_bytes(vec![slab0.clone(), slab1.clone()])
            .expect("store");
        let set = PagedSlabSet::open(vec![
            Arc::new(MockBytes::new(slab0)),
            Arc::new(MockBytes::new(slab1)),
        ])
        .expect("set");

        assert_eq!(set.slab_count(), store.slab_count());
        assert_eq!(set.drop_count(), store.drop_count());
        for id in [&id1, &id2, &id3] {
            assert!(set.contains(id));
            assert_eq!(set.plaintext_for(id), store.plaintext_for(id));
        }
        let missing = [0x99; 32];
        assert!(!set.contains(&missing));
        assert!(set.plaintext_for(&missing).is_none());

        // The SlabSource trait surface answers through the set.
        let as_trait: &dyn crate::slab_source::SlabSource = &set;
        assert_eq!(as_trait.slab_count(), 2);
        assert_eq!(as_trait.drop_count(), 3);
        assert_eq!(as_trait.plaintext_for(&id2), Some(Ok(b"second".to_vec())));
    }

    #[test]
    fn an_empty_reader_vec_is_an_empty_set() {
        let set = PagedSlabSet::open(Vec::new()).expect("empty set");
        assert_eq!(set.slab_count(), 0);
        assert_eq!(set.drop_count(), 0);
    }
}
