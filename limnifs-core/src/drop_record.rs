//! Drop record (spec §3.3, `bit-level/31-drop-record.md`).
//!
//! One 48-byte descriptor per drop in a slab, locating the drop's
//! bytes inside one of the slab's solid windows.

use crate::cursor::ManifestCursor;
use crate::error::CoreError;
use crate::slab::SlabHeader;
use limnifs_format::{DropId, Representation};

/// Width of a single drop record on the wire.
///
/// v0.2: extended from 48 to 49 bytes by adding `dict_id` (1 byte)
/// at the end. `dict_id` = 0xFF means "no dictionary"; 0..254
/// references an entry in the `dictionary_section` manifest section.
pub const DROP_RECORD_LEN: usize = 49;

/// Sentinel `dict_id` meaning "no dictionary used for this drop".
pub const NO_DICT: u8 = 0xFF;

/// Default per-drop plaintext-size ceiling. The spec's writer pipeline
/// typically produces drops in the 4–64 MiB range via `FastCDC`; larger
/// values are rejected unless the manifest overrides.
pub const DEFAULT_DROP_MAX_PLAINTEXT_BYTES: u64 = 64 * 1024 * 1024;

/// Parsed drop record.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DropRecord {
    pub drop_id: DropId,
    pub plaintext_len: u32,
    pub representation: Representation,
    pub solid_window_index: u8,
    pub offset_in_window: u32,
    pub len_in_window: u32,
    /// Dictionary id for dict-aided decompression.
    /// 0xFF = no dictionary; 0..254 = index into `dictionary_section`.
    pub dict_id: u8,
}

/// Parse a single drop record from the cursor's current position.
///
/// Performs the self-contained checks: buffer length, plaintext-size
/// ceiling, slab-vs-record cross-field consistency (representation's
/// aead must be 0 in a plaintext slab; ec must be 0 in a no-EC slab),
/// and the `offset_in_window + len_in_window` u32 overflow check.
///
/// Does NOT check `solid_window_index` against the slab's actual
/// solid-window count (the count is only known after parsing every
/// drop record in the slab). That cross-record check runs at the
/// slab-walker layer.
///
/// # Errors
///
/// - [`CoreError::TooShort`] if the cursor has fewer than 48 bytes.
/// - [`CoreError::Corrupt`] if `plaintext_len` exceeds `max_plaintext`,
///   if the slab-vs-record AEAD/EC consistency rules are violated, or
///   if `offset_in_window + len_in_window` overflows u32.
pub fn parse_drop_record(
    cursor: &mut ManifestCursor<'_>,
    slab: &SlabHeader,
) -> Result<DropRecord, CoreError> {
    parse_drop_record_with_ceiling(cursor, slab, DEFAULT_DROP_MAX_PLAINTEXT_BYTES)
}

/// Same as [`parse_drop_record`] but with a caller-supplied ceiling
/// on `plaintext_len`. Used by readers that have parsed a manifest
/// with a non-default drop-size parameter.
///
/// # Errors
///
/// Inherits all errors from [`parse_drop_record`].
pub fn parse_drop_record_with_ceiling(
    cursor: &mut ManifestCursor<'_>,
    slab: &SlabHeader,
    max_plaintext: u64,
) -> Result<DropRecord, CoreError> {
    let drop_id_bytes = cursor.read_n(32)?;
    let mut drop_id_array = [0u8; 32];
    drop_id_array.copy_from_slice(drop_id_bytes);
    let drop_id = DropId::from_bytes(drop_id_array);

    let plaintext_len = cursor.read_u32_le()?;
    if u64::from(plaintext_len) > max_plaintext {
        return Err(CoreError::Corrupt {
            reason: format!("drop plaintext_len {plaintext_len} exceeds ceiling {max_plaintext}"),
        });
    }
    let repr_bytes = cursor.read_n(3)?;
    let representation = Representation::from_bytes([repr_bytes[0], repr_bytes[1], repr_bytes[2]]);
    if !slab.is_sealed() && representation.aead != 0x00 {
        return Err(CoreError::Corrupt {
            reason: format!(
                "drop record declares aead=0x{:02X} but slab is plaintext (crypto_hint=0)",
                representation.aead
            ),
        });
    }
    if !slab.has_erasure_coding() && representation.ec != 0x00 {
        return Err(CoreError::Corrupt {
            reason: format!(
                "drop record declares ec=0x{:02X} but slab has no EC (ec_descriptor=0)",
                representation.ec
            ),
        });
    }
    let solid_window_index = cursor.read_u8()?;
    let offset_in_window = cursor.read_u32_le()?;
    let len_in_window = cursor.read_u32_le()?;
    if offset_in_window.checked_add(len_in_window).is_none() {
        return Err(CoreError::Corrupt {
            reason: format!(
                "drop record offset_in_window {offset_in_window} + len_in_window {len_in_window} overflows u32"
            ),
        });
    }
    let dict_id = cursor.read_u8()?;
    Ok(DropRecord {
        drop_id,
        plaintext_len,
        representation,
        solid_window_index,
        offset_in_window,
        len_in_window,
        dict_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slab::parse_slab_header;
    use limnifs_format::{SlabId, SLAB_MAGIC};

    fn make_plaintext_slab_header() -> SlabHeader {
        SlabHeader {
            format_version: 1,
            slab_id: SlabId::new(0, [0; 32]),
            total_length: 4096,
            ec_descriptor: 0x00,
            crypto_hint: 0x00,
        }
    }

    fn make_sealed_ec_slab_header() -> SlabHeader {
        SlabHeader {
            format_version: 1,
            slab_id: SlabId::new(0, [0; 32]),
            total_length: 16384,
            ec_descriptor: 0x01,
            crypto_hint: 0x01,
        }
    }

    fn make_drop_record_bytes(record: &DropRecord) -> [u8; DROP_RECORD_LEN] {
        let mut bytes = [0u8; DROP_RECORD_LEN];
        bytes[..32].copy_from_slice(record.drop_id.as_bytes());
        bytes[32..36].copy_from_slice(&record.plaintext_len.to_le_bytes());
        bytes[36..39].copy_from_slice(&record.representation.to_bytes());
        bytes[39] = record.solid_window_index;
        bytes[40..44].copy_from_slice(&record.offset_in_window.to_le_bytes());
        bytes[44..48].copy_from_slice(&record.len_in_window.to_le_bytes());
        bytes[48] = record.dict_id;
        bytes
    }

    fn sample_record() -> DropRecord {
        DropRecord {
            drop_id: DropId::from_bytes([0x55; 32]),
            plaintext_len: 1024,
            representation: Representation::STORE_PLAINTEXT,
            solid_window_index: 0,
            offset_in_window: 0,
            len_in_window: 1024,
            dict_id: NO_DICT,
        }
    }

    #[test]
    fn parses_store_plaintext_drop() {
        let slab = make_plaintext_slab_header();
        let record = sample_record();
        let bytes = make_drop_record_bytes(&record);
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_drop_record(&mut cursor, &slab).expect("plaintext drop parses");
        assert_eq!(parsed, record);
        assert_eq!(cursor.position(), DROP_RECORD_LEN);
    }

    #[test]
    fn parses_lz4_sealed_drop() {
        let slab = make_sealed_ec_slab_header();
        let record = DropRecord {
            drop_id: DropId::from_bytes([0x77; 32]),
            plaintext_len: 4096,
            representation: Representation::new(0x01, 0x01, 0x01),
            solid_window_index: 1,
            offset_in_window: 2048,
            len_in_window: 1024,
            dict_id: NO_DICT,
        };
        let bytes = make_drop_record_bytes(&record);
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_drop_record(&mut cursor, &slab).expect("sealed drop parses");
        assert_eq!(parsed, record);
    }

    #[test]
    fn rejects_short_buffer() {
        let slab = make_plaintext_slab_header();
        let bytes = [0u8; 40];
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_drop_record(&mut cursor, &slab) {
            Err(CoreError::TooShort { .. }) => {}
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn rejects_plaintext_len_above_ceiling() {
        let slab = make_plaintext_slab_header();
        let mut record = sample_record();
        record.plaintext_len = u32::MAX;
        let bytes = make_drop_record_bytes(&record);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_drop_record(&mut cursor, &slab) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("ceiling"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_aead_in_plaintext_slab() {
        let slab = make_plaintext_slab_header();
        let mut record = sample_record();
        record.representation = Representation::new(0x00, 0x01, 0x00);
        let bytes = make_drop_record_bytes(&record);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_drop_record(&mut cursor, &slab) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("aead"), "got: {reason}");
                assert!(reason.contains("plaintext"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_ec_in_no_ec_slab() {
        let slab = make_plaintext_slab_header();
        let mut record = sample_record();
        record.representation = Representation::new(0x00, 0x00, 0x01);
        let bytes = make_drop_record_bytes(&record);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_drop_record(&mut cursor, &slab) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("ec"), "got: {reason}");
                assert!(reason.contains("no EC"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_offset_len_overflow() {
        let slab = make_plaintext_slab_header();
        let mut record = sample_record();
        record.offset_in_window = u32::MAX;
        record.len_in_window = 1;
        let bytes = make_drop_record_bytes(&record);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_drop_record(&mut cursor, &slab) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("overflows u32"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn parses_after_a_real_slab_header() {
        let mut bytes = Vec::new();
        let mut header_bytes = [0u8; 56];
        header_bytes[..4].copy_from_slice(&SLAB_MAGIC);
        header_bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
        // slab_id ordinal 7, hash 0xAA
        header_bytes[6..14].copy_from_slice(&7u64.to_le_bytes());
        for byte in &mut header_bytes[14..46] {
            *byte = 0xAA;
        }
        header_bytes[46..54].copy_from_slice(&8192u64.to_le_bytes());
        // ec_descriptor=0, crypto_hint=0
        bytes.extend_from_slice(&header_bytes);

        let record = sample_record();
        bytes.extend_from_slice(&make_drop_record_bytes(&record));

        let mut cursor = ManifestCursor::new(&bytes);
        let slab_parsed = parse_slab_header(&mut cursor).expect("slab header parses");
        let record_parsed = parse_drop_record(&mut cursor, &slab_parsed).expect("drop parses");
        assert_eq!(record_parsed, record);
        assert_eq!(cursor.position(), 56 + DROP_RECORD_LEN);
    }
}
