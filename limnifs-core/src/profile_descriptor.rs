//! Profile descriptor manifest section.
//!
//! Records which overhead layers were active at write time, so any
//! reader can correctly handle the image regardless of which profile
//! or optimizations the writer used.
//!
//! ## Wire format
//!
//! ```text
//! +---+---+---+---+---+---+---+---+---+
//! | version (1) | name_len (1) | name (name_len) |
//! +---+---+---+---+---+---+---+---+---+
//! | flags (1)                         |
//! +---+---+---+---+---+---+---+---+---+
//! ```
//!
//! Flags bits:
//! - bit 0: BLAKE3 hashing enabled (DropId is content-addressed)
//! - bit 1: cross-file dedup enabled
//! - bit 2: content classification enabled
//! - bit 3: integrity verify (Merkle root) enabled
//! - bit 4: RW mode (vs RO)
//! - bit 5: auto-turnover enabled

#![allow(warnings)]

use crate::cursor::ManifestCursor;
use crate::error::CoreError;

pub const PROFILE_DESCRIPTOR_SECTION_VERSION: u8 = 1;
const FLAG_BLAKE3: u8 = 0x01;
const FLAG_DEDUP: u8 = 0x02;
const FLAG_CLASSIFY: u8 = 0x04;
const FLAG_VERIFY: u8 = 0x08;
const FLAG_RW: u8 = 0x10;
const FLAG_AUTO_TURNOVER: u8 = 0x20;
const MAX_NAME_LEN: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileDescriptor {
    pub version: u8,
    pub profile_name: Option<String>,
    pub blake3_hashing: bool,
    pub cross_file_dedup: bool,
    pub content_classification: bool,
    pub integrity_verify: bool,
    pub read_write: bool,
    pub auto_turnover: bool,
}

impl Default for ProfileDescriptor {
    fn default() -> Self {
        Self {
            version: PROFILE_DESCRIPTOR_SECTION_VERSION,
            profile_name: None,
            blake3_hashing: true,
            cross_file_dedup: true,
            content_classification: true,
            integrity_verify: true,
            read_write: false,
            auto_turnover: false,
        }
    }
}

pub fn parse_profile_descriptor(
    cursor: &mut ManifestCursor<'_>,
) -> Result<ProfileDescriptor, CoreError> {
    let section_version = cursor.read_u8()?;
    if section_version != PROFILE_DESCRIPTOR_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "profile_descriptor version {section_version} (supported: {PROFILE_DESCRIPTOR_SECTION_VERSION})"
            ),
        });
    }

    let name_len = usize::from(cursor.read_u8()?);
    let profile_name = if name_len == 0 || name_len > MAX_NAME_LEN {
        None
    } else {
        Some(
            String::from_utf8(cursor.read_n(name_len)?.to_vec()).map_err(|_| {
                CoreError::Corrupt {
                    reason: "profile_descriptor: name is not valid UTF-8".into(),
                }
            })?,
        )
    };

    let flags = cursor.read_u8()?;

    Ok(ProfileDescriptor {
        version: section_version,
        profile_name,
        blake3_hashing: (flags & FLAG_BLAKE3) != 0,
        cross_file_dedup: (flags & FLAG_DEDUP) != 0,
        content_classification: (flags & FLAG_CLASSIFY) != 0,
        integrity_verify: (flags & FLAG_VERIFY) != 0,
        read_write: (flags & FLAG_RW) != 0,
        auto_turnover: (flags & FLAG_AUTO_TURNOVER) != 0,
    })
}

pub fn encode_profile_descriptor(desc: &ProfileDescriptor, out: &mut Vec<u8>) {
    out.push(PROFILE_DESCRIPTOR_SECTION_VERSION);

    let name_bytes = desc.profile_name.as_deref().unwrap_or("").as_bytes();
    let name_len = u8::try_from(name_bytes.len()).unwrap_or(0);
    out.push(name_len);
    if name_len > 0 {
        out.extend_from_slice(&name_bytes[..name_len as usize]);
    }

    let mut flags = 0u8;
    if desc.blake3_hashing {
        flags |= FLAG_BLAKE3;
    }
    if desc.cross_file_dedup {
        flags |= FLAG_DEDUP;
    }
    if desc.content_classification {
        flags |= FLAG_CLASSIFY;
    }
    if desc.integrity_verify {
        flags |= FLAG_VERIFY;
    }
    if desc.read_write {
        flags |= FLAG_RW;
    }
    if desc.auto_turnover {
        flags |= FLAG_AUTO_TURNOVER;
    }
    out.push(flags);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_full() {
        let original = ProfileDescriptor {
            version: PROFILE_DESCRIPTOR_SECTION_VERSION,
            profile_name: Some("competitive".into()),
            blake3_hashing: true,
            cross_file_dedup: true,
            content_classification: true,
            integrity_verify: true,
            read_write: false,
            auto_turnover: false,
        };
        let mut encoded = Vec::new();
        encode_profile_descriptor(&original, &mut encoded);

        let mut cursor = ManifestCursor::new(&encoded);
        let parsed = parse_profile_descriptor(&mut cursor).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn round_trip_rw() {
        let original = ProfileDescriptor {
            version: PROFILE_DESCRIPTOR_SECTION_VERSION,
            profile_name: Some("max-write-rw".into()),
            blake3_hashing: false,
            cross_file_dedup: false,
            content_classification: false,
            integrity_verify: false,
            read_write: true,
            auto_turnover: true,
        };
        let mut encoded = Vec::new();
        encode_profile_descriptor(&original, &mut encoded);

        let mut cursor = ManifestCursor::new(&encoded);
        let parsed = parse_profile_descriptor(&mut cursor).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn round_trip_no_name() {
        let original = ProfileDescriptor {
            version: PROFILE_DESCRIPTOR_SECTION_VERSION,
            profile_name: None,
            ..ProfileDescriptor::default()
        };
        let mut encoded = Vec::new();
        encode_profile_descriptor(&original, &mut encoded);

        let mut cursor = ManifestCursor::new(&encoded);
        let parsed = parse_profile_descriptor(&mut cursor).expect("parse");
        assert_eq!(parsed, original);
    }
}
