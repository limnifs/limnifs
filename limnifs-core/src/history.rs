//! History section (spec §5.9, `bit-level/40-history.md`).
//!
//! Append-only log of operations applied to derive this image from
//! its inputs. Every image MUST have at least one history entry (the
//! `build` op that produced it).

use crate::cursor::ManifestCursor;
use crate::error::CoreError;
use limnifs_format::ManifestRoot;

/// Current layout version of this section.
pub const HISTORY_SECTION_VERSION: u8 = 1;

/// Width of the fixed prefix of the section (version byte + u32 LE count).
const PREFIX_LEN: usize = 1 + 4;

/// Width of the fixed prefix of each entry (op + timestamp + `input_count`,
/// before any inputs or params bytes).
const ENTRY_FIXED_LEN: usize = 1 + 8 + 4;

/// Default ceiling on the per-entry `params` blob length.
pub const DEFAULT_HISTORY_PARAMS_MAX_BYTES: u32 = 4 * 1024;

/// Sentinel value for `op` indicating an extended (post-v1) opcode
/// follows. Readers in v1 reject with `UnsupportedFeature`.
pub const OP_EXTENDED: u8 = 0xFF;

/// Operation kind, per spec §5.9.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum HistoryOp {
    Build = 0x01,
    Delta = 0x02,
    Flatten = 0x03,
    Turnover = 0x04,
    Deepen = 0x05,
}

impl HistoryOp {
    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(Self::Build),
            0x02 => Some(Self::Delta),
            0x03 => Some(Self::Flatten),
            0x04 => Some(Self::Turnover),
            0x05 => Some(Self::Deepen),
            _ => None,
        }
    }

    #[must_use]
    pub const fn to_byte(self) -> u8 {
        self as u8
    }
}

/// One history entry: one operation plus its inputs and parameters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryEntry {
    pub op: HistoryOp,
    pub timestamp_ns: u64,
    pub inputs: Vec<ManifestRoot>,
    pub params: Vec<u8>,
}

/// Parsed history section.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
}

impl History {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Parse the history section from the cursor's current position. Uses
/// the default 4 KiB per-entry params ceiling.
///
/// # Errors
///
/// - [`CoreError::UnsupportedFeature`] if `section_version` is not 1,
///   or if any entry's `op` is `0xFF` (extended opcode, post-v1), or
///   if any entry's `op` is in the reserved range `0x06`–`0xFE`.
/// - [`CoreError::Corrupt`] if `entry_count` is zero (every image has
///   a build entry), or if any entry's `params_len` exceeds the
///   ceiling.
/// - [`CoreError::TooShort`] if the cursor has fewer bytes than the
///   declared structure requires.
pub fn parse_history(cursor: &mut ManifestCursor<'_>) -> Result<History, CoreError> {
    parse_history_with_ceiling(cursor, DEFAULT_HISTORY_PARAMS_MAX_BYTES)
}

/// Same as [`parse_history`] but with a caller-supplied per-entry
/// params byte ceiling.
///
/// # Errors
///
/// Inherits all errors from [`parse_history`].
///
/// # Panics
///
/// Panics if the per-entry minimum width somehow overflows `usize`.
/// This is a compile-time constant expression (`13 + 4`); the panic
/// is unreachable on any supported platform.
pub fn parse_history_with_ceiling(
    cursor: &mut ManifestCursor<'_>,
    max_params_bytes: u32,
) -> Result<History, CoreError> {
    let section_version = cursor.read_u8()?;
    if section_version != HISTORY_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "history section version {section_version} (supported: {HISTORY_SECTION_VERSION})"
            ),
        });
    }
    let raw_count = cursor.read_u32_le()?;
    let entry_count = usize::try_from(raw_count).map_err(|_| CoreError::Corrupt {
        reason: format!("history entry count {raw_count} exceeds usize"),
    })?;
    if entry_count == 0 {
        return Err(CoreError::Corrupt {
            reason: "history entry_count is 0 (every image must have at least the build entry)"
                .into(),
        });
    }
    // DoS check: each entry needs at least ENTRY_FIXED_LEN + 4 bytes
    // (params_len field) before any inputs or params bodies. Reject
    // before allocating. The min_entry_width is a compile-time
    // constant (13 + 4 = 17) so checked_add is statically Some.
    let min_entry_width = ENTRY_FIXED_LEN + 4;
    let min_section_size =
        entry_count
            .checked_mul(min_entry_width)
            .ok_or_else(|| CoreError::Corrupt {
                reason: format!("history entry count {entry_count} overflows usize"),
            })?;
    if cursor.remaining_len() < min_section_size {
        return Err(CoreError::TooShort {
            have: cursor.remaining_len(),
            need: min_section_size,
        });
    }
    let mut entries = Vec::with_capacity(entry_count);
    for index in 0..entry_count {
        let entry = parse_history_entry_with_ceiling(cursor, max_params_bytes).map_err(|err| {
            // Annotate the error with the entry index.
            match err {
                CoreError::Corrupt { reason } => CoreError::Corrupt {
                    reason: format!("history entry {index}: {reason}"),
                },
                CoreError::UnsupportedFeature { feature } => CoreError::UnsupportedFeature {
                    feature: format!("history entry {index}: {feature}"),
                },
                other => other,
            }
        })?;
        entries.push(entry);
    }
    let _ = PREFIX_LEN; // documented constant; nothing to emit
    Ok(History { entries })
}

fn parse_history_entry_with_ceiling(
    cursor: &mut ManifestCursor<'_>,
    max_params_bytes: u32,
) -> Result<HistoryEntry, CoreError> {
    let op_byte = cursor.read_u8()?;
    if op_byte == OP_EXTENDED {
        return Err(CoreError::UnsupportedFeature {
            feature: "history op 0xFF (extended opcode, post-v1)".into(),
        });
    }
    let op = HistoryOp::from_byte(op_byte).ok_or_else(|| CoreError::UnsupportedFeature {
        feature: format!("history op 0x{op_byte:02X} is reserved (not in 0x01..0x05)"),
    })?;
    let timestamp_ns = cursor.read_u64_le()?;
    let raw_input_count = cursor.read_u32_le()?;
    let input_count = usize::try_from(raw_input_count).map_err(|_| CoreError::Corrupt {
        reason: format!("history input_count {raw_input_count} exceeds usize"),
    })?;
    let inputs_size = input_count
        .checked_mul(32)
        .ok_or_else(|| CoreError::Corrupt {
            reason: format!("history input_count {input_count} overflows usize when scaled by 32"),
        })?;
    let inputs_bytes = cursor.read_n(inputs_size)?;
    let mut inputs = Vec::with_capacity(input_count);
    for chunk in inputs_bytes.chunks_exact(32) {
        let mut root = [0u8; 32];
        root.copy_from_slice(chunk);
        inputs.push(ManifestRoot::from_bytes(root));
    }
    let params_len = cursor.read_u32_le()?;
    if params_len > max_params_bytes {
        return Err(CoreError::Corrupt {
            reason: format!("history params_len {params_len} exceeds ceiling {max_params_bytes}"),
        });
    }
    let params_len_us = usize::try_from(params_len).map_err(|_| CoreError::Corrupt {
        reason: format!("history params_len {params_len} exceeds usize"),
    })?;
    let params = cursor.read_n_owned(params_len_us)?;
    Ok(HistoryEntry {
        op,
        timestamp_ns,
        inputs,
        params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_history_entry_bytes(
        op: u8,
        timestamp_ns: u64,
        inputs: &[ManifestRoot],
        params: &[u8],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(op);
        bytes.extend_from_slice(&timestamp_ns.to_le_bytes());
        let input_count = u32::try_from(inputs.len()).expect("input count fits u32");
        bytes.extend_from_slice(&input_count.to_le_bytes());
        for input in inputs {
            bytes.extend_from_slice(input.as_bytes());
        }
        let params_len = u32::try_from(params.len()).expect("params len fits u32");
        bytes.extend_from_slice(&params_len.to_le_bytes());
        bytes.extend_from_slice(params);
        bytes
    }

    fn make_history_bytes(version: u8, entries: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(version);
        let count = u32::try_from(entries.len()).expect("count fits u32");
        bytes.extend_from_slice(&count.to_le_bytes());
        for entry in entries {
            bytes.extend_from_slice(entry);
        }
        bytes
    }

    fn sample_root(byte: u8) -> ManifestRoot {
        ManifestRoot::from_bytes([byte; 32])
    }

    #[test]
    fn parses_single_deterministic_build() {
        let entry = make_history_entry_bytes(0x01, 0, &[], &[]);
        let bytes = make_history_bytes(HISTORY_SECTION_VERSION, &[entry]);
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_history(&mut cursor).expect("single build parses");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed.entries[0].op, HistoryOp::Build);
        assert_eq!(parsed.entries[0].timestamp_ns, 0);
        assert!(parsed.entries[0].inputs.is_empty());
        assert!(parsed.entries[0].params.is_empty());
        assert_eq!(cursor.position(), bytes.len());
    }

    #[test]
    fn parses_delta_with_one_parent_and_params() {
        let parent = sample_root(0xAB);
        let params = vec![0xCC; 64];
        let entry = make_history_entry_bytes(0x02, 1_735_000_000_000_000_000, &[parent], &params);
        let bytes = make_history_bytes(HISTORY_SECTION_VERSION, &[entry]);
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_history(&mut cursor).expect("delta parses");
        assert_eq!(parsed.entries[0].op, HistoryOp::Delta);
        assert_eq!(parsed.entries[0].timestamp_ns, 1_735_000_000_000_000_000);
        assert_eq!(parsed.entries[0].inputs.len(), 1);
        assert_eq!(parsed.entries[0].inputs[0], parent);
        assert_eq!(parsed.entries[0].params, params);
    }

    #[test]
    fn parses_multi_op_chain() {
        let parent = sample_root(0x11);
        let build_entry = make_history_entry_bytes(0x01, 0, &[], &[]);
        let deepen_entry = make_history_entry_bytes(0x05, 1000, &[parent], &[0xDD; 16]);
        let bytes = make_history_bytes(HISTORY_SECTION_VERSION, &[build_entry, deepen_entry]);
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_history(&mut cursor).expect("chain parses");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.entries[0].op, HistoryOp::Build);
        assert_eq!(parsed.entries[1].op, HistoryOp::Deepen);
        assert_eq!(parsed.entries[1].inputs.len(), 1);
    }

    #[test]
    fn parses_every_opcode() {
        for (byte, expected) in [
            (0x01, HistoryOp::Build),
            (0x02, HistoryOp::Delta),
            (0x03, HistoryOp::Flatten),
            (0x04, HistoryOp::Turnover),
            (0x05, HistoryOp::Deepen),
        ] {
            let entry = make_history_entry_bytes(byte, 0, &[], &[]);
            let bytes = make_history_bytes(HISTORY_SECTION_VERSION, &[entry]);
            let mut cursor = ManifestCursor::new(&bytes);
            let parsed =
                parse_history(&mut cursor).unwrap_or_else(|e| panic!("op {byte:02X}: {e:?}"));
            assert_eq!(parsed.entries[0].op, expected);
        }
    }

    #[test]
    fn rejects_unknown_section_version() {
        let entry = make_history_entry_bytes(0x01, 0, &[], &[]);
        let bytes = make_history_bytes(7, &[entry]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_history(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("version 7"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_zero_entry_count() {
        let bytes = make_history_bytes(HISTORY_SECTION_VERSION, &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_history(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("entry_count"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_reserved_opcode() {
        let entry = make_history_entry_bytes(0x10, 0, &[], &[]);
        let bytes = make_history_bytes(HISTORY_SECTION_VERSION, &[entry]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_history(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("reserved"), "got: {feature}");
                assert!(feature.contains("0x10"));
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_extended_opcode() {
        let entry = make_history_entry_bytes(OP_EXTENDED, 0, &[], &[]);
        let bytes = make_history_bytes(HISTORY_SECTION_VERSION, &[entry]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_history(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("0xFF"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_params_above_default_ceiling() {
        let oversized = vec![0xEE; (DEFAULT_HISTORY_PARAMS_MAX_BYTES as usize) + 1];
        let entry = make_history_entry_bytes(0x01, 0, &[], &oversized);
        let bytes = make_history_bytes(HISTORY_SECTION_VERSION, &[entry]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_history(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("ceiling"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn custom_ceiling_accepts_oversized_params() {
        let oversized = vec![0xFF; (DEFAULT_HISTORY_PARAMS_MAX_BYTES as usize) + 100];
        let entry = make_history_entry_bytes(0x01, 0, &[], &oversized);
        let bytes = make_history_bytes(HISTORY_SECTION_VERSION, &[entry]);
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_history_with_ceiling(&mut cursor, 2 * DEFAULT_HISTORY_PARAMS_MAX_BYTES)
            .expect("custom ceiling accepts");
        assert_eq!(parsed.entries[0].params, oversized);
    }

    #[test]
    fn rejects_entry_count_overrunning_buffer() {
        let mut bytes = Vec::new();
        bytes.push(HISTORY_SECTION_VERSION);
        bytes.extend_from_slice(&10u32.to_le_bytes()); // 10 entries declared
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_history(&mut cursor) {
            Err(CoreError::TooShort { have, need }) => {
                assert_eq!(have, 0);
                assert_eq!(need, 10 * (ENTRY_FIXED_LEN + 4));
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn entry_error_is_annotated_with_index() {
        // Entry 0 is valid; entry 1 has a reserved opcode.
        let good = make_history_entry_bytes(0x01, 0, &[], &[]);
        let bad = make_history_entry_bytes(0x20, 0, &[], &[]);
        let bytes = make_history_bytes(HISTORY_SECTION_VERSION, &[good, bad]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_history(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("entry 1"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_truncated_prefix() {
        // Valid version + partial entry_count.
        let bytes = [HISTORY_SECTION_VERSION, 0, 0];
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_history(&mut cursor) {
            Err(CoreError::TooShort { .. }) => {}
            other => panic!("expected TooShort, got {other:?}"),
        }
    }
}
