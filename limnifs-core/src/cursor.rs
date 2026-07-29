//! Cursor over a manifest byte slice.
//!
//! Centralises bounds checking and position tracking so that every
//! section parser ([`crate::header`], [`crate::feature_flags`], …)
//! reads from the same abstraction. The cursor is a thin wrapper over
//! `&[u8]`; all accessors are zero-cost after inlining.
//!
//! Parsers take `&mut ManifestCursor<'_>` and return their typed
//! result or a [`crate::error::CoreError`]. Advancing the cursor on
//! success is the parser's responsibility; on error the cursor's
//! position is unspecified.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::error::CoreError;

/// A bounded cursor over the bytes of a manifest (or slab).
#[derive(Debug, Clone, Copy)]
pub struct ManifestCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ManifestCursor<'a> {
    /// Construct from a byte slice. The cursor starts at position 0.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Construct at a non-zero starting position. Used by parsers that
    /// resume mid-buffer (e.g. after a header read by an outer reader).
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::TooShort`] if `start > bytes.len()`.
    pub fn at_start(bytes: &'a [u8], start: usize) -> Result<Self, CoreError> {
        if start > bytes.len() {
            return Err(CoreError::TooShort {
                have: bytes.len(),
                need: start,
            });
        }
        Ok(Self { bytes, pos: start })
    }

    /// Current byte position from the start of the underlying slice.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.pos
    }

    /// Number of bytes remaining at and after the cursor position.
    #[must_use]
    pub const fn remaining_len(&self) -> usize {
        if self.pos >= self.bytes.len() {
            0
        } else {
            self.bytes.len() - self.pos
        }
    }

    /// The bytes that remain unread.
    #[must_use]
    pub fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.pos..]
    }

    /// Advance past `n` bytes without inspecting them. Useful for
    /// reserved fields once validated.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::TooShort`] if `n` exceeds the remaining
    /// bytes.
    pub fn skip(&mut self, n: usize) -> Result<(), CoreError> {
        self.read_n(n).map(|_| ())
    }

    /// Read one byte and advance.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::TooShort`] if no bytes remain.
    pub fn read_u8(&mut self) -> Result<u8, CoreError> {
        let bytes = self.read_n(1)?;
        Ok(bytes[0])
    }

    /// Read two bytes as a little-endian u16 and advance.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::TooShort`] if fewer than 2 bytes remain.
    pub fn read_u16_le(&mut self) -> Result<u16, CoreError> {
        let bytes = self.read_n(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// Read four bytes as a little-endian u32 and advance.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::TooShort`] if fewer than 4 bytes remain.
    pub fn read_u32_le(&mut self) -> Result<u32, CoreError> {
        let bytes = self.read_n(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read eight bytes as a little-endian u64 and advance.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::TooShort`] if fewer than 8 bytes remain.
    pub fn read_u64_le(&mut self) -> Result<u64, CoreError> {
        let bytes = self.read_n(8)?;
        let arr: [u8; 8] = bytes.try_into().map_err(|_| CoreError::Corrupt {
            reason: "internal: 8-byte slice did not fit [u8; 8]".into(),
        })?;
        Ok(u64::from_le_bytes(arr))
    }

    /// Read exactly 4 bytes as a magic constant and advance.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::TooShort`] if fewer than 4 bytes remain.
    pub fn read_magic(&mut self) -> Result<[u8; 4], CoreError> {
        let bytes = self.read_n(4)?;
        let mut out = [0u8; 4];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    /// Read `n` bytes and advance. The returned slice borrows from the
    /// cursor's underlying buffer.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::TooShort`] if fewer than `n` bytes remain.
    /// Returns [`CoreError::Corrupt`] if `n` overflows `usize` when
    /// added to the current position.
    pub fn read_n(&mut self, n: usize) -> Result<&'a [u8], CoreError> {
        let end = self.pos.checked_add(n).ok_or_else(|| CoreError::Corrupt {
            reason: format!("read of {n} bytes overflows usize at position {}", self.pos),
        })?;
        if end > self.bytes.len() {
            return Err(CoreError::TooShort {
                have: self.remaining_len(),
                need: n,
            });
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// Read `n` bytes and copy them into a fresh `Vec<u8>`. Use this
    /// when the caller needs to own the bytes (e.g. to outlive the
    /// cursor or to mutate). For read-only access, prefer
    /// [`Self::read_n`].
    ///
    /// # Errors
    ///
    /// Inherits errors from [`Self::read_n`].
    pub fn read_n_owned(&mut self, n: usize) -> Result<Vec<u8>, CoreError> {
        self.read_n(n).map(std::borrow::ToOwned::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_starts_at_zero() {
        let cursor = ManifestCursor::new(&[0u8; 32]);
        assert_eq!(cursor.position(), 0);
        assert_eq!(cursor.remaining_len(), 32);
    }

    #[test]
    fn at_start_resumes_mid_buffer() {
        let bytes = [1, 2, 3, 4, 5];
        let mut cursor = ManifestCursor::at_start(&bytes, 2).expect("offset 2 is valid");
        assert_eq!(cursor.position(), 2);
        assert_eq!(cursor.read_u8().unwrap(), 3);
    }

    #[test]
    fn at_start_rejects_offset_past_end() {
        let bytes = [1, 2, 3];
        match ManifestCursor::at_start(&bytes, 4) {
            Err(CoreError::TooShort { have, need }) => {
                assert_eq!((have, need), (3, 4));
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn read_u8_advances_and_returns_byte() {
        let bytes = [0x42, 0x99];
        let mut cursor = ManifestCursor::new(&bytes);
        assert_eq!(cursor.read_u8().unwrap(), 0x42);
        assert_eq!(cursor.read_u8().unwrap(), 0x99);
        assert_eq!(cursor.position(), 2);
        assert!(cursor.read_u8().is_err());
    }

    #[test]
    fn read_u16_le_decodes_little_endian() {
        let bytes = [0x34, 0x12];
        let mut cursor = ManifestCursor::new(&bytes);
        assert_eq!(cursor.read_u16_le().unwrap(), 0x1234);
        assert_eq!(cursor.position(), 2);
    }

    #[test]
    fn read_u32_le_decodes_little_endian() {
        let bytes = [0x78, 0x56, 0x34, 0x12];
        let mut cursor = ManifestCursor::new(&bytes);
        assert_eq!(cursor.read_u32_le().unwrap(), 0x1234_5678);
    }

    #[test]
    fn read_u64_le_decodes_little_endian() {
        let bytes = [0xEF, 0xCD, 0xAB, 0x90, 0x78, 0x56, 0x34, 0x12];
        let mut cursor = ManifestCursor::new(&bytes);
        assert_eq!(cursor.read_u64_le().unwrap(), 0x1234_5678_90AB_CDEF);
    }

    #[test]
    fn read_n_returns_slice_and_advances() {
        let bytes = [0, 1, 2, 3, 4, 5];
        let mut cursor = ManifestCursor::new(&bytes);
        let slice = cursor.read_n(3).unwrap();
        assert_eq!(slice, &[0, 1, 2]);
        assert_eq!(cursor.position(), 3);
        assert_eq!(cursor.remaining(), &[3, 4, 5]);
    }

    #[test]
    fn read_n_owned_returns_independent_vec() {
        let bytes = [9, 8, 7];
        let mut cursor = ManifestCursor::new(&bytes);
        let owned = cursor.read_n_owned(2).unwrap();
        assert_eq!(owned, vec![9, 8]);
        assert_eq!(cursor.position(), 2);
    }

    #[test]
    fn read_magic_returns_four_byte_array() {
        let bytes = *b"LMFS____";
        let mut cursor = ManifestCursor::new(&bytes);
        assert_eq!(cursor.read_magic().unwrap(), *b"LMFS");
        assert_eq!(cursor.position(), 4);
    }

    #[test]
    fn skip_advances_without_inspecting() {
        let bytes = [0u8; 8];
        let mut cursor = ManifestCursor::new(&bytes);
        cursor.skip(4).unwrap();
        assert_eq!(cursor.position(), 4);
        assert_eq!(cursor.remaining_len(), 4);
    }

    #[test]
    fn read_past_end_yields_too_short() {
        let bytes = [0u8; 2];
        let mut cursor = ManifestCursor::new(&bytes);
        cursor.skip(2).unwrap();
        match cursor.read_u8() {
            Err(CoreError::TooShort { have, need }) => {
                assert_eq!((have, need), (0, 1));
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn read_n_with_overflow_yields_corrupt() {
        let bytes = [0u8; 4];
        let mut cursor = ManifestCursor::new(&bytes);
        cursor.skip(1).unwrap(); // advance so pos + usize::MAX overflows
        let huge = usize::MAX;
        match cursor.read_n(huge) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("overflows"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn remaining_len_is_zero_at_end() {
        let bytes = [0u8; 4];
        let mut cursor = ManifestCursor::new(&bytes);
        cursor.skip(4).unwrap();
        assert_eq!(cursor.remaining_len(), 0);
        assert!(cursor.remaining().is_empty());
    }
}
