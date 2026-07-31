//! Epoch operations — the tree mutations an epoch applies.
//!
//! Each operation is a single byte opcode followed by opcode-specific
//! fields. Paths are length-prefixed UTF-8 (u32 LE length + bytes).

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::cursor::ManifestCursor;
use crate::epoch::EpochId;
use crate::error::CoreError;

/// Opcodes for [`EpochOp`].
///
/// **Wire stability:** once an opcode ships, its binary value never
/// changes. New operations get new opcodes; existing opcodes are never
/// renumbered. This matches the spec's per-section versioning rule.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum EpochOpKind {
    /// Add a new file at `path` referencing `drop_id`.
    Add = 0x00,
    /// Remove the file or empty directory at `path`.
    Remove = 0x01,
    /// Replace the content of an existing file at `path` with `drop_id`.
    Modify = 0x02,
    /// Change the permission bits of `path` to `mode`.
    Chmod = 0x03,
    /// Rename `from` to `to`. The source must exist; the target must not.
    Rename = 0x04,
    /// Create a new directory at `path`.
    Mkdir = 0x05,
    /// Remove an empty directory at `path`.
    Rmdir = 0x06,
}

impl EpochOpKind {
    /// Construct from a raw opcode byte.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::Corrupt`] if `raw` is not a known opcode.
    pub fn from_u8(raw: u8) -> Result<Self, CoreError> {
        Ok(match raw {
            0x00 => Self::Add,
            0x01 => Self::Remove,
            0x02 => Self::Modify,
            0x03 => Self::Chmod,
            0x04 => Self::Rename,
            0x05 => Self::Mkdir,
            0x06 => Self::Rmdir,
            other => {
                return Err(CoreError::Corrupt {
                    reason: format!("epoch op: unknown opcode 0x{other:02X}"),
                });
            }
        })
    }
}

/// One operation in an epoch. Each variant carries the fields its opcode
/// needs; the `kind()` method exposes the opcode for `match` dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EpochOp {
    Add {
        path: String,
        drop_id: EpochId,
        len: u64,
        mode: u32,
        mtime: u64,
    },
    Remove {
        path: String,
    },
    Modify {
        path: String,
        drop_id: EpochId,
        len: u64,
        mtime: u64,
    },
    Chmod {
        path: String,
        mode: u32,
    },
    Rename {
        from: String,
        to: String,
    },
    Mkdir {
        path: String,
        mode: u32,
    },
    Rmdir {
        path: String,
    },
}

impl EpochOp {
    /// The opcode for this operation.
    #[must_use]
    pub fn kind(&self) -> EpochOpKind {
        match self {
            Self::Add { .. } => EpochOpKind::Add,
            Self::Remove { .. } => EpochOpKind::Remove,
            Self::Modify { .. } => EpochOpKind::Modify,
            Self::Chmod { .. } => EpochOpKind::Chmod,
            Self::Rename { .. } => EpochOpKind::Rename,
            Self::Mkdir { .. } => EpochOpKind::Mkdir,
            Self::Rmdir { .. } => EpochOpKind::Rmdir,
        }
    }
}

/// Serialise the operations section: `ops_count: u32` followed by each
/// operation in deterministic order.
///
/// # Panics
///
/// Panics if the path count exceeds u32 (impossible — bounded by metadata).
#[must_use]
pub fn write_op_section(ops: &[EpochOp]) -> Vec<u8> {
    let mut out = Vec::new();
    let count = u32::try_from(ops.len()).expect("ops count fits u32");
    out.extend_from_slice(&count.to_le_bytes());
    for op in ops {
        write_one_op(op, &mut out);
    }
    out
}

fn write_one_op(op: &EpochOp, out: &mut Vec<u8>) {
    out.push(op.kind() as u8);
    match op {
        EpochOp::Add {
            path,
            drop_id,
            len,
            mode,
            mtime,
        } => {
            write_path(path, out);
            out.extend_from_slice(drop_id);
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&mode.to_le_bytes());
            out.extend_from_slice(&mtime.to_le_bytes());
        }
        // Remove and Rmdir share the same serialised form (path only).
        // The opcodes differ, which is what distinguishes them on the wire.
        #[allow(clippy::match_same_arms)]
        EpochOp::Remove { path } => {
            write_path(path, out);
        }
        EpochOp::Modify {
            path,
            drop_id,
            len,
            mtime,
        } => {
            write_path(path, out);
            out.extend_from_slice(drop_id);
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&mtime.to_le_bytes());
        }
        // Chmod and Mkdir share the same serialised form (path + mode).
        // The opcodes differ, which is what distinguishes them on the wire.
        #[allow(clippy::match_same_arms)]
        EpochOp::Chmod { path, mode } => {
            write_path(path, out);
            out.extend_from_slice(&mode.to_le_bytes());
        }
        EpochOp::Rename { from, to } => {
            write_path(from, out);
            write_path(to, out);
        }
        EpochOp::Mkdir { path, mode } => {
            write_path(path, out);
            out.extend_from_slice(&mode.to_le_bytes());
        }
        EpochOp::Rmdir { path } => {
            write_path(path, out);
        }
    }
}

fn write_path(path: &str, out: &mut Vec<u8>) {
    let bytes = path.as_bytes();
    let len = u32::try_from(bytes.len()).expect("path length fits u32");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
}

/// Parse the operations section from a cursor.
///
/// # Errors
///
/// Returns [`CoreError::Corrupt`] on truncation, unknown opcode, or
/// non-UTF-8 path.
pub fn parse_op_section(cursor: &mut ManifestCursor<'_>) -> Result<Vec<EpochOp>, CoreError> {
    let count = cursor.read_u32_le().map_err(|e| CoreError::Corrupt {
        reason: format!("epoch ops: truncated before count: {e}"),
    })?;
    let count = usize::try_from(count).map_err(|_| CoreError::Corrupt {
        reason: format!("epoch ops: count {count} exceeds usize"),
    })?;
    let mut ops = Vec::with_capacity(count.min(1024));
    for i in 0..count {
        let opcode = cursor.read_u8().map_err(|e| CoreError::Corrupt {
            reason: format!("epoch ops: op {i}: truncated before opcode: {e}"),
        })?;
        let kind = EpochOpKind::from_u8(opcode)?;
        let op = parse_one_op(kind, cursor, i)?;
        ops.push(op);
    }
    Ok(ops)
}

fn parse_one_op(
    kind: EpochOpKind,
    cursor: &mut ManifestCursor<'_>,
    index: usize,
) -> Result<EpochOp, CoreError> {
    fn read_path(cursor: &mut ManifestCursor<'_>, index: usize) -> Result<String, CoreError> {
        let len = cursor.read_u32_le().map_err(|e| CoreError::Corrupt {
            reason: format!("epoch ops: op {index}: truncated before path length: {e}"),
        })?;
        let len = usize::try_from(len).map_err(|_| CoreError::Corrupt {
            reason: format!("epoch ops: op {index}: path length {len} exceeds usize"),
        })?;
        let bytes = cursor.read_n(len).map_err(|e| CoreError::Corrupt {
            reason: format!("epoch ops: op {index}: truncated within path: {e}"),
        })?;
        String::from_utf8(bytes.to_vec()).map_err(|e| CoreError::Corrupt {
            reason: format!("epoch ops: op {index}: path is not UTF-8: {e}"),
        })
    }

    fn read_id(
        cursor: &mut ManifestCursor<'_>,
        index: usize,
        label: &str,
    ) -> Result<EpochId, CoreError> {
        let bytes = cursor.read_n(32).map_err(|e| CoreError::Corrupt {
            reason: format!("epoch ops: op {index}: truncated before {label}: {e}"),
        })?;
        let mut id = [0u8; 32];
        id.copy_from_slice(bytes);
        Ok(id)
    }

    Ok(match kind {
        EpochOpKind::Add => EpochOp::Add {
            path: read_path(cursor, index)?,
            drop_id: read_id(cursor, index, "drop_id")?,
            len: cursor.read_u64_le().map_err(|e| CoreError::Corrupt {
                reason: format!("epoch ops: op {index}: truncated before len: {e}"),
            })?,
            mode: cursor.read_u32_le().map_err(|e| CoreError::Corrupt {
                reason: format!("epoch ops: op {index}: truncated before mode: {e}"),
            })?,
            mtime: cursor.read_u64_le().map_err(|e| CoreError::Corrupt {
                reason: format!("epoch ops: op {index}: truncated before mtime: {e}"),
            })?,
        },
        EpochOpKind::Remove => EpochOp::Remove {
            path: read_path(cursor, index)?,
        },
        EpochOpKind::Modify => EpochOp::Modify {
            path: read_path(cursor, index)?,
            drop_id: read_id(cursor, index, "drop_id")?,
            len: cursor.read_u64_le().map_err(|e| CoreError::Corrupt {
                reason: format!("epoch ops: op {index}: truncated before len: {e}"),
            })?,
            mtime: cursor.read_u64_le().map_err(|e| CoreError::Corrupt {
                reason: format!("epoch ops: op {index}: truncated before mtime: {e}"),
            })?,
        },
        EpochOpKind::Chmod => EpochOp::Chmod {
            path: read_path(cursor, index)?,
            mode: cursor.read_u32_le().map_err(|e| CoreError::Corrupt {
                reason: format!("epoch ops: op {index}: truncated before mode: {e}"),
            })?,
        },
        EpochOpKind::Rename => EpochOp::Rename {
            from: read_path(cursor, index)?,
            to: read_path(cursor, index)?,
        },
        EpochOpKind::Mkdir => EpochOp::Mkdir {
            path: read_path(cursor, index)?,
            mode: cursor.read_u32_le().map_err(|e| CoreError::Corrupt {
                reason: format!("epoch ops: op {index}: truncated before mode: {e}"),
            })?,
        },
        EpochOpKind::Rmdir => EpochOp::Rmdir {
            path: read_path(cursor, index)?,
        },
    })
}
