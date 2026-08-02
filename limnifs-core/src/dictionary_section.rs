//! Dictionary section manifest module.
//!
//! Stores trained compression dictionaries (currently ZSTD) so the
//! reader can reconstruct dict-aided decompression without external
//! configuration. Each dictionary is keyed by a `dict_id` (0..254)
//! that [`DropRecord::dict_id`][crate::drop_record::DropRecord] references.
//!
//! ## Wire format
//!
//! ```text
//! +---+---+---+---+---+
//! | version (1) | dict_count (1) |
//! +---+---+---+---+---+
//! per dict:
//!   codec_id (1) | class_id (1) | dict_len (4 LE) | dict_data (dict_len)
//! ```

use crate::cursor::ManifestCursor;
use crate::error::CoreError;

pub const DICTIONARY_SECTION_VERSION: u8 = 1;
const MAX_DICT_COUNT: u8 = 255;
const MAX_DICT_SIZE: u32 = 1024 * 1024;

/// Sentinel `dict_id` meaning "no dictionary". Matches the default
/// `dict_id` in [`crate::drop_record::DropRecord`].
pub const NO_DICT: u8 = 0xFF;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictionarySection {
    pub version: u8,
    pub dicts: Vec<Dictionary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dictionary {
    pub codec_id: u8,
    pub class_id: u8,
    pub data: Vec<u8>,
}

pub fn parse_dictionary_section(
    cursor: &mut ManifestCursor<'_>,
) -> Result<DictionarySection, CoreError> {
    let section_version = cursor.read_u8()?;
    if section_version != DICTIONARY_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "dictionary_section version {section_version} (supported: {DICTIONARY_SECTION_VERSION})"
            ),
        });
    }

    let dict_count = cursor.read_u8()?;
    let mut dicts = Vec::with_capacity(usize::from(dict_count));

    for _ in 0..dict_count {
        let codec_id = cursor.read_u8()?;
        let class_id = cursor.read_u8()?;
        let dict_len = cursor.read_u32_le()?;
        if dict_len > MAX_DICT_SIZE {
            return Err(CoreError::Corrupt {
                reason: format!("dictionary len {dict_len} exceeds cap {MAX_DICT_SIZE}"),
            });
        }
        let data = cursor
            .read_n(usize::try_from(dict_len).unwrap_or(0))?
            .to_vec();
        dicts.push(Dictionary {
            codec_id,
            class_id,
            data,
        });
    }

    Ok(DictionarySection {
        version: section_version,
        dicts,
    })
}

pub fn encode_dictionary_section(section: &DictionarySection, out: &mut Vec<u8>) {
    out.push(DICTIONARY_SECTION_VERSION);
    let count = u8::try_from(section.dicts.len()).unwrap_or(MAX_DICT_COUNT);
    out.push(count);

    for dict in &section.dicts {
        out.push(dict.codec_id);
        out.push(dict.class_id);
        let len = u32::try_from(dict.data.len()).unwrap_or(MAX_DICT_SIZE);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&dict.data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let original = DictionarySection {
            version: DICTIONARY_SECTION_VERSION,
            dicts: vec![
                Dictionary {
                    codec_id: 0x02, // ZSTD
                    class_id: 0x00, // text
                    data: vec![0x42; 1024],
                },
                Dictionary {
                    codec_id: 0x02,
                    class_id: 0x01, // code
                    data: vec![0x55; 512],
                },
            ],
        };
        let mut encoded = Vec::new();
        encode_dictionary_section(&original, &mut encoded);

        let mut cursor = ManifestCursor::new(&encoded);
        let parsed = parse_dictionary_section(&mut cursor).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn round_trip_empty() {
        let original = DictionarySection {
            version: DICTIONARY_SECTION_VERSION,
            dicts: vec![],
        };
        let mut encoded = Vec::new();
        encode_dictionary_section(&original, &mut encoded);

        let mut cursor = ManifestCursor::new(&encoded);
        let parsed = parse_dictionary_section(&mut cursor).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn rejects_oversized_dict() {
        let mut encoded = vec![DICTIONARY_SECTION_VERSION, 1, 0x02, 0x00];
        encoded.extend_from_slice(&(MAX_DICT_SIZE + 1).to_le_bytes());
        let mut cursor = ManifestCursor::new(&encoded);
        assert!(parse_dictionary_section(&mut cursor).is_err());
    }

    #[test]
    fn no_dict_sentinel() {
        assert_eq!(NO_DICT, 0xFF);
    }
}
