//! Epoch drop payload — the new content-addressed drops an epoch introduces.
//!
//! Each drop carries:
//!
//! - `drop_id`: `BLAKE3(plaintext)` — the content-addressed identity.
//!   Matches `DropId` semantics elsewhere in `LimniFS`: codec and payload
//!   are representations, never identity.
//! - `codec`: which compression codec produced `payload`.
//! - `plaintext_len`: the uncompressed size, used for allocation hints
//!   and integrity verification.
//! - `payload`: the compressed bytes (or the raw plaintext if codec is store).

use crate::cursor::ManifestCursor;
use crate::epoch::EpochId;
use crate::error::CoreError;

/// One new drop introduced by an epoch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpochDrop {
    /// `BLAKE3(plaintext)` — content-addressed identity, never affected
    /// by codec or encryption choices.
    pub drop_id: EpochId,
    /// Codec id from [`crate::codec`]: STORE / LZ4 / ZSTD / etc.
    pub codec: u8,
    /// Uncompressed size in bytes. The decoder MUST produce exactly
    /// this many bytes.
    pub plaintext_len: u32,
    /// The compressed bytes (or raw plaintext if codec is STORE).
    pub payload: Vec<u8>,
}

/// Serialise the drops section: `drops_count: u32` followed by each drop.
///
/// # Panics
///
/// Panics if the drop count or any payload length exceeds u32. Both are
/// bounded by upstream validation.
#[must_use]
pub fn write_drop_section(drops: &[EpochDrop]) -> Vec<u8> {
    let mut out = Vec::new();
    let count = u32::try_from(drops.len()).expect("drops count fits u32");
    out.extend_from_slice(&count.to_le_bytes());
    for drop in drops {
        out.extend_from_slice(&drop.drop_id);
        out.push(drop.codec);
        out.extend_from_slice(&drop.plaintext_len.to_le_bytes());
        let payload_len = u32::try_from(drop.payload.len()).expect("payload fits u32");
        out.extend_from_slice(&payload_len.to_le_bytes());
        out.extend_from_slice(&drop.payload);
    }
    out
}

/// Parse the drops section from a cursor.
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] on truncation or oversize length fields.
pub fn parse_drop_section(cursor: &mut ManifestCursor<'_>) -> Result<Vec<EpochDrop>, CoreError> {
    let count = cursor.read_u32_le().map_err(|e| CoreError::Corrupt {
        reason: format!("epoch drops: truncated before count: {e}"),
    })?;
    let count = usize::try_from(count).map_err(|_| CoreError::Corrupt {
        reason: format!("epoch drops: count {count} exceeds usize"),
    })?;
    let mut drops = Vec::with_capacity(count.min(4096));
    for i in 0..count {
        let drop = parse_one_drop(cursor, i)?;
        drops.push(drop);
    }
    Ok(drops)
}

fn parse_one_drop(cursor: &mut ManifestCursor<'_>, index: usize) -> Result<EpochDrop, CoreError> {
    let id_bytes = cursor.read_n(32).map_err(|e| CoreError::Corrupt {
        reason: format!("epoch drops: drop {index}: truncated before drop_id: {e}"),
    })?;
    let mut drop_id = [0u8; 32];
    drop_id.copy_from_slice(id_bytes);

    let codec = cursor.read_u8().map_err(|e| CoreError::Corrupt {
        reason: format!("epoch drops: drop {index}: truncated before codec: {e}"),
    })?;

    let plaintext_len = cursor.read_u32_le().map_err(|e| CoreError::Corrupt {
        reason: format!("epoch drops: drop {index}: truncated before plaintext_len: {e}"),
    })?;

    let payload_len = cursor.read_u32_le().map_err(|e| CoreError::Corrupt {
        reason: format!("epoch drops: drop {index}: truncated before payload_len: {e}"),
    })?;
    let payload_len = usize::try_from(payload_len).map_err(|_| CoreError::Corrupt {
        reason: format!("epoch drops: drop {index}: payload_len {payload_len} exceeds usize"),
    })?;

    let payload_bytes = cursor.read_n(payload_len).map_err(|e| CoreError::Corrupt {
        reason: format!("epoch drops: drop {index}: truncated within payload: {e}"),
    })?;

    Ok(EpochDrop {
        drop_id,
        codec,
        plaintext_len,
        payload: payload_bytes.to_vec(),
    })
}
