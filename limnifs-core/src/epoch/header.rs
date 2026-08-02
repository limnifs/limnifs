//! Epoch header — the fixed-size prefix of every epoch file.

use crate::cursor::ManifestCursor;
use crate::epoch::{EpochId, EPOCH_FORMAT_VERSION};
use crate::error::CoreError;

/// Magic bytes "LIME" (Limni Epoch) at the start of every epoch file.
pub const EPOCH_MAGIC: u32 = 0x4C49_4D45;

/// Fixed header size: magic(4) + version(2) + flags(2) + parent(32) +
/// base(32) + seq(8) + `ops_hash(32)` + `drops_hash(32)` + `own_epoch_id(32)`
/// + timestamp(8) + reserved(16) = 200 bytes.
pub const HEADER_SIZE: usize = 200;

/// Bit-flag set on the epoch header. Matches the wire format's u16 flags
/// field.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct EpochFlags(u16);

impl EpochFlags {
    /// Construct empty flags (no signature, no timestamp, not sealed).
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Bit 0: epoch carries an Ed25519 signature (task 08).
    #[must_use]
    pub const fn signed(self) -> Self {
        Self(self.0 | 0x0001)
    }

    /// Bit 1: epoch's `timestamp_unix` field is populated (task 09).
    #[must_use]
    pub const fn timestamped(self) -> Self {
        Self(self.0 | 0x0002)
    }

    /// Bit 2: epoch has been sealed against further commits (task 11).
    #[must_use]
    pub const fn sealed(self) -> Self {
        Self(self.0 | 0x0004)
    }

    /// Raw u16 wire value.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Construct from a raw u16 read off the wire.
    #[must_use]
    pub const fn from_u16(raw: u16) -> Self {
        Self(raw)
    }
}

/// The fixed-size header prefixing every epoch file.
///
/// `own_epoch_id` is the BLAKE3 hash of the full epoch bytes (with
/// `own_epoch_id` itself zeroed during hashing). It is the epoch's
/// content-addressed identity and is what the next epoch records as
/// `parent_epoch_id`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpochHeader {
    /// Format version; matches [`crate::epoch::EPOCH_FORMAT_VERSION`].
    pub version: u16,
    /// Bit flags (signed, timestamped, sealed, ...).
    pub flags: EpochFlags,
    /// BLAKE3 of the parent epoch; all-zero for an epoch-0 base.
    pub parent_epoch_id: EpochId,
    /// BLAKE3 manifest root of the base `.lim` image this chain overlays.
    pub base_image_root: EpochId,
    /// Monotonic counter: 0 for the base image, 1+ for commits on top.
    pub epoch_sequence: u64,
    /// BLAKE3 of the operations section (`ops_count` + ops).
    pub ops_hash: [u8; 32],
    /// BLAKE3 of the drops section (`drops_count` + drops).
    pub drops_hash: [u8; 32],
    /// BLAKE3 of the full epoch with this field zeroed. The epoch's
    /// content-addressed identity.
    pub own_epoch_id: EpochId,
    /// Unix timestamp of commit; 0 if untimestamped.
    pub timestamp_unix: u64,
}

impl Default for EpochHeader {
    fn default() -> Self {
        Self {
            version: EPOCH_FORMAT_VERSION,
            flags: EpochFlags::empty(),
            parent_epoch_id: [0u8; 32],
            base_image_root: [0u8; 32],
            epoch_sequence: 0,
            ops_hash: [0u8; 32],
            drops_hash: [0u8; 32],
            own_epoch_id: [0u8; 32],
            timestamp_unix: 0,
        }
    }
}

/// Write the header to `out` in canonical (little-endian) form. Does NOT
/// validate `own_epoch_id` — callers populate it via [`crate::epoch::Epoch::build`].
pub fn write_epoch_header(header: &EpochHeader, out: &mut Vec<u8>) {
    out.extend_from_slice(&EPOCH_MAGIC.to_le_bytes());
    out.extend_from_slice(&header.version.to_le_bytes());
    out.extend_from_slice(&header.flags.as_u16().to_le_bytes());
    out.extend_from_slice(&header.parent_epoch_id);
    out.extend_from_slice(&header.base_image_root);
    out.extend_from_slice(&header.epoch_sequence.to_le_bytes());
    out.extend_from_slice(&header.ops_hash);
    out.extend_from_slice(&header.drops_hash);
    out.extend_from_slice(&header.own_epoch_id);
    out.extend_from_slice(&header.timestamp_unix.to_le_bytes());
    out.extend_from_slice(&[0u8; 16]); // reserved
    debug_assert_eq!(out.len(), HEADER_SIZE, "header size invariant");
}

/// Parse the header from a cursor. Advances the cursor past the header.
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] if:
/// - the magic bytes don't match [`EPOCH_MAGIC`]
/// - the version is unsupported (> [`EPOCH_FORMAT_VERSION`])
/// - the cursor runs out of bytes
pub fn parse_epoch_header(cursor: &mut ManifestCursor<'_>) -> Result<EpochHeader, CoreError> {
    fn copy_id(cursor: &mut ManifestCursor<'_>, label: &str) -> Result<[u8; 32], CoreError> {
        let bytes = cursor.read_n(32).map_err(|e| CoreError::Corrupt {
            reason: format!("epoch header: truncated before {label}: {e}"),
        })?;
        let mut id = [0u8; 32];
        id.copy_from_slice(bytes);
        Ok(id)
    }

    let magic = cursor.read_u32_le().map_err(|e| CoreError::Corrupt {
        reason: format!("epoch header: truncated before magic: {e}"),
    })?;
    if magic != EPOCH_MAGIC {
        return Err(CoreError::Corrupt {
            reason: format!(
                "epoch header: magic 0x{magic:08X} does not match LIME (0x{EPOCH_MAGIC:08X})"
            ),
        });
    }

    let version = cursor.read_u16_le().map_err(|e| CoreError::Corrupt {
        reason: format!("epoch header: truncated before version: {e}"),
    })?;
    if version > EPOCH_FORMAT_VERSION {
        return Err(CoreError::Corrupt {
            reason: format!(
                "epoch header: version {version} exceeds supported {EPOCH_FORMAT_VERSION}"
            ),
        });
    }

    let flags_raw = cursor.read_u16_le().map_err(|e| CoreError::Corrupt {
        reason: format!("epoch header: truncated before flags: {e}"),
    })?;

    let parent_epoch_id = copy_id(cursor, "parent_epoch_id")?;
    let base_image_root = copy_id(cursor, "base_image_root")?;

    let epoch_sequence = cursor.read_u64_le().map_err(|e| CoreError::Corrupt {
        reason: format!("epoch header: truncated before epoch_sequence: {e}"),
    })?;

    let ops_hash = copy_id(cursor, "ops_hash")?;
    let drops_hash = copy_id(cursor, "drops_hash")?;
    let own_epoch_id = copy_id(cursor, "own_epoch_id")?;

    let timestamp_unix = cursor.read_u64_le().map_err(|e| CoreError::Corrupt {
        reason: format!("epoch header: truncated before timestamp: {e}"),
    })?;

    cursor.skip(16).map_err(|e| CoreError::Corrupt {
        reason: format!("epoch header: truncated before end of reserved: {e}"),
    })?;

    Ok(EpochHeader {
        version,
        flags: EpochFlags::from_u16(flags_raw),
        parent_epoch_id,
        base_image_root,
        epoch_sequence,
        ops_hash,
        drops_hash,
        own_epoch_id,
        timestamp_unix,
    })
}
