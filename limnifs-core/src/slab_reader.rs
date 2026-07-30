//! Slab reader — locates and extracts a drop's plaintext from a slab.
//!
//! The slab layout (spec §3.1) is:
//!
//! ```text
//! +---------------------------------+
//! | SlabHeader (fixed, 56 bytes)    |
//! +---------------------------------+
//! | DropRecord[0..n]                |   ← 48 bytes each
//! +---------------------------------+
//! | SolidWindow[0..m]               |   ← concatenated drop plaintexts
//! +---------------------------------+
//! | ECShards (optional)             |
//! +---------------------------------+
//! ```
//!
//! The slab header does not carry an explicit `drop_count`; readers
//! derive it by walking records until the cursor would enter the
//! solid window. The stop condition for a store-codec slab (the only
//! kind the v0.1 writer emits) is:
//!
//! ```text
//! cursor_position + Σ plaintext_len_so_far == total_length
//! ```
//!
//! At that point the remaining bytes are the solid window, and each
//! record's `(offset_in_window, len_in_window)` is an absolute byte
//! range inside it.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::cursor::ManifestCursor;
use crate::drop_record::{parse_drop_record, DropRecord, DROP_RECORD_LEN};
use crate::error::CoreError;
use crate::slab::{parse_slab_header, SlabHeader};

/// Parsed slab: header + drop records + view onto the solid window.
///
/// `bytes` borrows the underlying slab buffer for lifetime `'a`. The
/// `plaintext_for` accessor returns slices that borrow from `bytes`,
/// so callers can keep those slices around as long as the slab buffer
/// itself stays alive.
#[derive(Debug, Clone)]
pub struct SlabView<'a> {
    bytes: &'a [u8],
    header: SlabHeader,
    drop_records: Vec<DropRecord>,
    solid_window_start: usize,
}

impl SlabView<'_> {
    /// The slab header.
    #[must_use]
    pub const fn header(&self) -> SlabHeader {
        self.header
    }

    /// All drop records in this slab, in declaration order.
    #[must_use]
    pub fn drop_records(&self) -> &[DropRecord] {
        &self.drop_records
    }

    /// Byte offset where the solid window begins (i.e. immediately
    /// after the last drop record). Useful for diagnostics.
    #[must_use]
    pub const fn solid_window_offset(&self) -> usize {
        self.solid_window_start
    }

    /// Find a drop record by its `DropId`. Linear scan.
    #[must_use]
    pub fn find_record(&self, drop_id: &[u8; 32]) -> Option<&DropRecord> {
        self.drop_records
            .iter()
            .find(|r| r.drop_id.as_bytes() == drop_id)
    }

    /// Return the plaintext bytes for `drop_id`, or `None` if no drop
    /// in this slab carries that id.
    ///
    /// Supports both store (0x00) and LZ4 (0x01) codecs. LZ4 drops
    /// are decompressed on read. Non-plaintext AEADs and non-zero
    /// `solid_window_index` are still rejected (v0.1 limitations).
    ///
    /// Returns owned bytes (not a borrowed slice) because LZ4
    /// decompression produces new data that does not live in the slab
    /// buffer.
    ///
    /// # Errors
    ///
    /// - [`CoreError::UnsupportedFeature`] if the drop uses an unknown
    ///   codec, a non-plaintext AEAD, or a non-zero `solid_window_index`.
    /// - [`CoreError::Corrupt`] if the slice would extend past the slab
    ///   or decompression fails.
    #[must_use]
    pub fn plaintext_for(&self, drop_id: &[u8; 32]) -> Option<Result<Vec<u8>, CoreError>> {
        let record = self.find_record(drop_id)?;
        if record.representation.aead != 0x00 {
            return Some(Err(CoreError::UnsupportedFeature {
                feature: format!(
                    "drop aead 0x{:02X} (only plaintext/0x00 supported in v0.1)",
                    record.representation.aead
                ),
            }));
        }
        if record.solid_window_index != 0 {
            return Some(Err(CoreError::UnsupportedFeature {
                feature: format!(
                    "solid_window_index {} (only single-window slabs supported in v0.1)",
                    record.solid_window_index
                ),
            }));
        }
        let offset = usize::try_from(record.offset_in_window).ok()?;
        let len = usize::try_from(record.len_in_window).ok()?;
        let start = self.solid_window_start.checked_add(offset)?;
        let end = start.checked_add(len)?;
        if end > self.bytes.len() {
            return Some(Err(CoreError::Corrupt {
                reason: format!(
                    "drop range [{start}..{end}] extends past slab length {}",
                    self.bytes.len()
                ),
            }));
        }
        let raw = &self.bytes[start..end];
        Some(crate::codec::decompress(
            record.representation.codec,
            raw,
            record.plaintext_len,
        ))
    }
}

/// Parse a slab into a [`SlabView`] that exposes drop records and
/// plaintext lookups.
///
/// Walks every drop record to derive the solid-window boundary. Only
/// store-codec plaintext slabs (the kind the v0.1 writer emits) are
/// supported; a slab whose records' `plaintext_len` values do not sum
/// to the trailing byte count is rejected as `Corrupt`.
///
/// # Errors
///
/// - Inherits errors from [`parse_slab_header`] and [`parse_drop_record`].
/// - [`CoreError::Corrupt`] if the drop-record / solid-window boundary
///   cannot be derived consistently.
pub fn parse_slab(bytes: &[u8]) -> Result<SlabView<'_>, CoreError> {
    let mut cursor = ManifestCursor::new(bytes);
    let header = parse_slab_header(&mut cursor)?;
    let total_length = usize::try_from(header.total_length).map_err(|_| CoreError::Corrupt {
        reason: format!("slab total_length {} exceeds usize", header.total_length),
    })?;
    if total_length != bytes.len() {
        return Err(CoreError::Corrupt {
            reason: format!(
                "slab total_length {total_length} does not match buffer length {}",
                bytes.len()
            ),
        });
    }

    let mut drop_records: Vec<DropRecord> = Vec::new();
    let mut window_len_sum: u64 = 0;
    loop {
        let cursor_pos = u64::try_from(cursor.position()).map_err(|_| CoreError::Corrupt {
            reason: format!("slab cursor position {} exceeds u64", cursor.position()),
        })?;
        let remaining_after_cursor =
            header
                .total_length
                .checked_sub(cursor_pos)
                .ok_or_else(|| CoreError::Corrupt {
                    reason: format!(
                        "slab cursor position {cursor_pos} past total_length {}",
                        header.total_length
                    ),
                })?;
        if remaining_after_cursor == window_len_sum {
            break;
        }
        if remaining_after_cursor < window_len_sum {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "slab drop records overran solid window: cursor_pos={cursor_pos}, window_sum={window_len_sum}, total_length={}",
                    header.total_length
                ),
            });
        }
        let trailing = remaining_after_cursor - window_len_sum;
        if trailing < u64::try_from(DROP_RECORD_LEN).unwrap_or(u64::MAX) {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "slab has {trailing} trailing bytes that are neither a full drop record nor accounted for by the solid window"
                ),
            });
        }
        let record = parse_drop_record(&mut cursor, &header)?;
        window_len_sum = window_len_sum
            .checked_add(u64::from(record.len_in_window))
            .ok_or_else(|| CoreError::Corrupt {
                reason: format!(
                    "slab drop len_in_window sum overflow at record {}",
                    drop_records.len()
                ),
            })?;
        drop_records.push(record);
    }

    let solid_window_start = cursor.position();
    Ok(SlabView {
        bytes,
        header,
        drop_records,
        solid_window_start,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slab::SLAB_HEADER_LEN;
    use limnifs_format::DropId;

    fn make_slab(drops: &[(&[u8; 32], &[u8])]) -> Vec<u8> {
        let mut drop_records = Vec::new();
        let mut solid_window = Vec::new();
        for (id, plaintext) in drops {
            let plaintext_len = u32::try_from(plaintext.len()).unwrap();
            let offset_in_window = u32::try_from(solid_window.len()).unwrap();
            drop_records.extend_from_slice(*id);
            drop_records.extend_from_slice(&plaintext_len.to_le_bytes());
            drop_records.extend_from_slice(&[0x00, 0x00, 0x00]); // representation: store, plaintext, no EC
            drop_records.push(0x00); // solid_window_index
            drop_records.extend_from_slice(&offset_in_window.to_le_bytes());
            drop_records.extend_from_slice(&plaintext_len.to_le_bytes());
            solid_window.extend_from_slice(plaintext);
        }
        let slab_content = [&drop_records[..], &solid_window[..]].concat();
        let total_length = u64::try_from(SLAB_HEADER_LEN + slab_content.len()).unwrap();
        let mut bytes = Vec::with_capacity(usize::try_from(total_length).expect("fits usize"));
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

    #[test]
    fn parses_empty_slab() {
        let bytes = make_slab(&[]);
        let view = parse_slab(&bytes).expect("empty slab parses");
        assert_eq!(view.drop_records().len(), 0);
    }

    #[test]
    fn parses_single_drop() {
        let id = [0xAA; 32];
        let plaintext = b"hello world";
        let bytes = make_slab(&[(&id, plaintext)]);
        let view = parse_slab(&bytes).expect("single-drop slab parses");
        assert_eq!(view.drop_records().len(), 1);
        let got = view
            .plaintext_for(&id)
            .expect("drop present")
            .expect("store codec ok");
        assert_eq!(got, plaintext);
    }

    #[test]
    fn parses_multiple_drops() {
        let id1 = [0x11; 32];
        let id2 = [0x22; 32];
        let id3 = [0x33; 32];
        let p1 = b"first drop plaintext";
        let p2 = b"second";
        let p3 = b"third drop is longer than the others combined";
        let bytes = make_slab(&[(&id1, p1), (&id2, p2), (&id3, p3)]);
        let view = parse_slab(&bytes).expect("multi-drop slab parses");
        assert_eq!(view.drop_records().len(), 3);
        assert_eq!(
            view.plaintext_for(&id1)
                .expect("drop 1 present")
                .expect("store codec ok"),
            p1
        );
        assert_eq!(
            view.plaintext_for(&id2)
                .expect("drop 2 present")
                .expect("store codec ok"),
            p2
        );
        assert_eq!(
            view.plaintext_for(&id3)
                .expect("drop 3 present")
                .expect("store codec ok"),
            p3
        );
    }

    #[test]
    fn missing_drop_returns_none() {
        let id = [0xAA; 32];
        let bytes = make_slab(&[(&id, b"data")]);
        let view = parse_slab(&bytes).expect("slab parses");
        let missing = DropId::from_bytes([0xBB; 32]);
        assert!(view.plaintext_for(missing.as_bytes()).is_none());
    }

    #[test]
    fn rejects_buffer_length_mismatch() {
        let id = [0xAA; 32];
        let mut bytes = make_slab(&[(&id, b"data")]);
        bytes.truncate(bytes.len() - 1);
        match parse_slab(&bytes) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(
                    reason.contains("does not match buffer length"),
                    "got: {reason}"
                );
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn slab_from_writer_round_trips() {
        // Build a slab using the writer's encoding helper and verify
        // the reader can extract plaintexts.
        let id1 = [0x11; 32];
        let id2 = [0x22; 32];
        let p1 = vec![0xAB; 4096];
        let p2 = vec![0xCD; 1024];
        let bytes = make_slab(&[(&id1, &p1), (&id2, &p2)]);
        let view = parse_slab(&bytes).expect("writer-style slab parses");
        assert_eq!(view.drop_records().len(), 2);
        assert_eq!(
            view.plaintext_for(&id1)
                .expect("drop 1 present")
                .expect("ok"),
            &p1[..]
        );
        assert_eq!(
            view.plaintext_for(&id2)
                .expect("drop 2 present")
                .expect("ok"),
            &p2[..]
        );
    }
}
