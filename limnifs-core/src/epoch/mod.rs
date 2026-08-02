//! Epoch format — writable images via content-addressed epoch chains.
//!
//! An epoch is a single commit on top of an immutable base `.lim` image.
//! Each epoch carries:
//!
//! - A header that links to its parent epoch and the base image root.
//! - An operations list (Add / Remove / Modify / Chmod / Rename / Mkdir / Rmdir).
//! - The new drops the operations reference.
//!
//! The epoch's identity is `BLAKE3(epoch_bytes)` — content-addressed, same
//! scheme as `DropId`. Epochs form a Merkle chain: each epoch's header
//! records its parent's id, so the chain is tamper-evident end-to-end.
//!
//! Replay (task 03) applies operations in chain order to reconstruct the
//! filesystem state at any epoch. Commit (task 04) diffs an overlay
//! directory against the current state and emits a new epoch file.
//!
//! ## Wire format
//!
//! All integers are little-endian. Paths are UTF-8, length-prefixed u32.
//!
//! ```text
//! EpochFile {
//!   magic:                 u32  = 0x4C494D45  // "LIME"
//!   version:               u16  = 1
//!   flags:                 u16  // bit 0=signed, 1=timestamped, 2=sealed
//!   parent_epoch_id:      [u8; 32]  // BLAKE3 of parent; zero for epoch 0
//!   base_image_root:      [u8; 32]  // BLAKE3 manifest root of base .lim
//!   epoch_sequence:        u64      // 0 for base, 1+ for commits
//!   ops_hash:             [u8; 32]  // BLAKE3 of operations section
//!   drops_hash:          [u8; 32]   // BLAKE3 of drops section
//!   own_epoch_id:        [u8; 32]   // BLAKE3 of everything above + sections
//!   timestamp_unix:       u64       // 0 if untimestamped
//!   reserved:            [u8; 16]   // alignment, future use
//!   // --- operations section ---
//!   ops_count:            u32
//!   ops:                 [EpochOp; ops_count]
//!   // --- drops section ---
//!   drops_count:          u32
//!   drops:              [EpochDrop; drops_count]
//! }
//! ```

mod drop_payload;
mod header;
mod ops;

pub use drop_payload::{parse_drop_section, write_drop_section, EpochDrop};
pub use header::{parse_epoch_header, write_epoch_header, EpochFlags, EpochHeader, EPOCH_MAGIC};
pub use ops::{parse_op_section, write_op_section, EpochOp, EpochOpKind};

use crate::error::CoreError;

/// The epoch format version this crate writes and reads.
pub const EPOCH_FORMAT_VERSION: u16 = 1;

/// The 32-byte BLAKE3 hash that identifies an epoch. Same width as
/// `DropId` and `ManifestRoot` — content-addressed throughout.
pub type EpochId = [u8; 32];

/// Compute the content-addressed id of an epoch by hashing its full
/// serialised bytes. This is the function that defines epoch identity.
#[must_use]
pub fn compute_epoch_id(epoch_bytes: &[u8]) -> EpochId {
    let mut id = [0u8; 32];
    id.copy_from_slice(blake3::hash(epoch_bytes).as_bytes());
    id
}

/// Compute the BLAKE3 hash of a section (operations or drops). Used to
/// populate `ops_hash` and `drops_hash` in the header before the header
/// itself is hashed for `own_epoch_id`.
#[must_use]
pub fn hash_section(bytes: &[u8]) -> [u8; 32] {
    let mut h = [0u8; 32];
    h.copy_from_slice(blake3::hash(bytes).as_bytes());
    h
}

/// A fully-parsed epoch: header + operations + drops. The `own_epoch_id`
/// in the header MUST equal `compute_epoch_id(self.to_bytes())`.
#[derive(Clone, Debug)]
pub struct Epoch {
    pub header: EpochHeader,
    pub ops: Vec<EpochOp>,
    pub drops: Vec<EpochDrop>,
}

impl Epoch {
    /// Serialise the epoch to its canonical bytes. The output is what
    /// `compute_epoch_id` hashes. Deterministic: same inputs ⇒ same bytes,
    /// across runs and machines.
    ///
    /// # Panics
    ///
    /// Panics if the path count or any path length exceeds u32. Both are
    /// bounded by the filesystem and validated upstream.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let ops_section = write_op_section(&self.ops);
        let drops_section = write_drop_section(&self.drops);
        let mut bytes = Vec::with_capacity(
            header::HEADER_SIZE + 4 + ops_section.len() + 4 + drops_section.len(),
        );
        write_epoch_header(&self.header, &mut bytes);
        bytes.extend_from_slice(&ops_section);
        bytes.extend_from_slice(&drops_section);
        bytes
    }

    /// The epoch's content-addressed id, computed from its canonical bytes.
    /// This is the value recorded in the next epoch's `parent_epoch_id`.
    #[must_use]
    pub fn id(&self) -> EpochId {
        let bytes = self.to_bytes();
        compute_epoch_id(&bytes)
    }

    /// Construct a new epoch from the given fields, computing the section
    /// hashes and `own_epoch_id` automatically. The caller does not need
    /// to populate `ops_hash`, `drops_hash`, or `own_epoch_id` in
    /// `header` — this function fills them.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::Corrupt`] if any operation or drop fails to
    /// serialise (e.g., path too long).
    #[must_use]
    pub fn build(mut header: EpochHeader, ops: Vec<EpochOp>, drops: Vec<EpochDrop>) -> Self {
        let ops_section = write_op_section(&ops);
        let drops_section = write_drop_section(&drops);
        header.ops_hash = hash_section(&ops_section);
        header.drops_hash = hash_section(&drops_section);

        // Compute own_epoch_id by hashing the full epoch bytes. We
        // temporarily zero own_epoch_id in the header, write the full
        // epoch, hash it, then store the hash.
        header.own_epoch_id = [0u8; 32];
        let mut bytes = Vec::with_capacity(
            header::HEADER_SIZE + 4 + ops_section.len() + 4 + drops_section.len(),
        );
        write_epoch_header(&header, &mut bytes);
        bytes.extend_from_slice(&ops_section);
        bytes.extend_from_slice(&drops_section);
        header.own_epoch_id = compute_epoch_id(&bytes);

        Self { header, ops, drops }
    }

    /// Parse a full epoch from bytes. Validates that `own_epoch_id` in the
    /// header matches the recomputed BLAKE3 of the full bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::Corrupt`] if:
    /// - the magic is wrong
    /// - the version is unsupported
    /// - any section is malformed
    /// - `own_epoch_id` does not match the recomputed hash (tamper check)
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CoreError> {
        let mut cursor = crate::cursor::ManifestCursor::new(bytes);
        let header = parse_epoch_header(&mut cursor)?;

        // Validate own_epoch_id by recomputing. Temporarily zero the
        // header's own_epoch_id field in the byte stream to match how
        // `build` computed it.
        let recomputed = {
            let mut tmp = bytes.to_vec();
            // own_epoch_id offset: magic(4) + version(2) + flags(2) +
            // parent(32) + base(32) + seq(8) + ops_hash(32) + drops_hash(32)
            // = 144. The field is 32 bytes wide.
            let offset = 4 + 2 + 2 + 32 + 32 + 8 + 32 + 32;
            tmp[offset..offset + 32].copy_from_slice(&[0u8; 32]);
            compute_epoch_id(&tmp)
        };
        if recomputed != header.own_epoch_id {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "epoch own_epoch_id mismatch: header claims {} but bytes hash to {}",
                    hex(&header.own_epoch_id),
                    hex(&recomputed)
                ),
            });
        }

        let ops = parse_op_section(&mut cursor)?;
        let drops = parse_drop_section(&mut cursor)?;
        Ok(Self { header, ops, drops })
    }
}

/// Format a 32-byte hash as lowercase hex for diagnostic messages.
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        write!(&mut s, "{b:02x}").expect("hex write");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_header() -> EpochHeader {
        EpochHeader {
            version: EPOCH_FORMAT_VERSION,
            flags: EpochFlags::empty(),
            parent_epoch_id: [0u8; 32],
            base_image_root: [0u8; 32],
            epoch_sequence: 1,
            ops_hash: [0u8; 32],
            drops_hash: [0u8; 32],
            own_epoch_id: [0u8; 32],
            timestamp_unix: 0,
        }
    }

    #[test]
    fn empty_epoch_round_trips() {
        let original = Epoch::build(empty_header(), Vec::new(), Vec::new());
        let bytes = original.to_bytes();
        let parsed = Epoch::from_bytes(&bytes).expect("parse");
        assert_eq!(parsed.header.version, EPOCH_FORMAT_VERSION);
        assert_eq!(parsed.header.epoch_sequence, 1);
        assert!(parsed.ops.is_empty());
        assert!(parsed.drops.is_empty());
        assert_eq!(parsed.header.own_epoch_id, original.header.own_epoch_id);
    }

    #[test]
    fn epoch_id_is_deterministic() {
        let epoch = Epoch::build(empty_header(), Vec::new(), Vec::new());
        let id1 = epoch.id();
        let id2 = epoch.id();
        assert_eq!(id1, id2, "epoch id must be deterministic");
    }

    #[test]
    fn ops_round_trip() {
        let drop_id = [0xAA; 32];
        let ops = vec![
            EpochOp::Add {
                path: "hello.txt".into(),
                drop_id,
                len: 11,
                mode: 0o644,
                mtime: 1_700_000_000,
            },
            EpochOp::Mkdir {
                path: "subdir".into(),
                mode: 0o755,
            },
            EpochOp::Remove {
                path: "old.txt".into(),
            },
        ];
        let epoch = Epoch::build(empty_header(), ops.clone(), Vec::new());
        let bytes = epoch.to_bytes();
        let parsed = Epoch::from_bytes(&bytes).expect("parse");
        assert_eq!(parsed.ops.len(), 3);
        assert!(matches!(parsed.ops[0].kind(), EpochOpKind::Add));
        assert!(matches!(parsed.ops[1].kind(), EpochOpKind::Mkdir));
        assert!(matches!(parsed.ops[2].kind(), EpochOpKind::Remove));
    }

    #[test]
    fn drops_round_trip() {
        let drop_id = [0xBB; 32];
        let drops = vec![EpochDrop {
            drop_id,
            codec: crate::codec::CODEC_STORE,
            plaintext_len: 5,
            payload: b"hello".to_vec(),
        }];
        let epoch = Epoch::build(empty_header(), Vec::new(), drops.clone());
        let bytes = epoch.to_bytes();
        let parsed = Epoch::from_bytes(&bytes).expect("parse");
        assert_eq!(parsed.drops.len(), 1);
        assert_eq!(parsed.drops[0].drop_id, drop_id);
        assert_eq!(parsed.drops[0].payload, b"hello");
        assert_eq!(parsed.drops[0].plaintext_len, 5);
    }

    #[test]
    fn tampered_bytes_rejected() {
        let epoch = Epoch::build(empty_header(), Vec::new(), Vec::new());
        let mut bytes = epoch.to_bytes();
        // Flip one bit near the end of the header.
        bytes[200] ^= 0x01;
        match Epoch::from_bytes(&bytes) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("mismatch"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn wrong_magic_rejected() {
        let epoch = Epoch::build(empty_header(), Vec::new(), Vec::new());
        let mut bytes = epoch.to_bytes();
        bytes[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        match Epoch::from_bytes(&bytes) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("magic"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn chain_links_via_parent_id() {
        let epoch1 = Epoch::build(empty_header(), Vec::new(), Vec::new());
        let id1 = epoch1.id();

        let mut header2 = empty_header();
        header2.epoch_sequence = 2;
        header2.parent_epoch_id = id1;
        let epoch2 = Epoch::build(header2, Vec::new(), Vec::new());

        let parsed2 = Epoch::from_bytes(&epoch2.to_bytes()).expect("parse");
        assert_eq!(parsed2.header.parent_epoch_id, id1);
        assert_eq!(parsed2.header.epoch_sequence, 2);
    }
}
