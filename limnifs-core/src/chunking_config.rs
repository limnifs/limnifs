//! Chunking config manifest section.
//!
//! Records the FastCDC parameters used at write time.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::cursor::ManifestCursor;
use crate::error::CoreError;

pub const CHUNKING_CONFIG_SECTION_VERSION: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkingConfig {
    pub version: u8,
    pub avg_chunk_size: u32,
    pub min_chunk_size: u32,
    pub max_chunk_size: u32,
}

pub fn parse_chunking_config(
    cursor: &mut ManifestCursor<'_>,
) -> Result<ChunkingConfig, CoreError> {
    let section_version = cursor.read_u8()?;
    if section_version != CHUNKING_CONFIG_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "chunking_config version {section_version} (supported: {CHUNKING_CONFIG_SECTION_VERSION})"
            ),
        });
    }

    let avg_chunk_size = cursor.read_u32_le()?;
    let min_chunk_size = cursor.read_u32_le()?;
    let max_chunk_size = cursor.read_u32_le()?;

    if min_chunk_size > avg_chunk_size {
        return Err(CoreError::Corrupt {
            reason: format!(
                "chunking_config: min_chunk_size ({min_chunk_size}) > avg_chunk_size ({avg_chunk_size})"
            ),
        });
    }
    if avg_chunk_size > max_chunk_size {
        return Err(CoreError::Corrupt {
            reason: format!(
                "chunking_config: avg_chunk_size ({avg_chunk_size}) > max_chunk_size ({max_chunk_size})"
            ),
        });
    }

    Ok(ChunkingConfig {
        version: section_version,
        avg_chunk_size,
        min_chunk_size,
        max_chunk_size,
    })
}

pub fn encode_chunking_config(config: &ChunkingConfig, out: &mut Vec<u8>) {
    out.push(CHUNKING_CONFIG_SECTION_VERSION);
    out.extend_from_slice(&config.avg_chunk_size.to_le_bytes());
    out.extend_from_slice(&config.min_chunk_size.to_le_bytes());
    out.extend_from_slice(&config.max_chunk_size.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let original = ChunkingConfig {
            version: CHUNKING_CONFIG_SECTION_VERSION,
            avg_chunk_size: 8192,
            min_chunk_size: 1024,
            max_chunk_size: 65_536,
        };
        let mut encoded = Vec::new();
        encode_chunking_config(&original, &mut encoded);

        let mut cursor = ManifestCursor::new(&encoded);
        let parsed = parse_chunking_config(&mut cursor).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn rejects_invalid_relation() {
        let encoded = {
            let mut out = Vec::new();
            out.push(CHUNKING_CONFIG_SECTION_VERSION);
            out.extend_from_slice(&4000u32.to_le_bytes()); // avg
            out.extend_from_slice(&8000u32.to_le_bytes()); // min > avg
            out.extend_from_slice(&16000u32.to_le_bytes());
            out
        };
        let mut cursor = ManifestCursor::new(&encoded);
        assert!(parse_chunking_config(&mut cursor).is_err());
    }
}
