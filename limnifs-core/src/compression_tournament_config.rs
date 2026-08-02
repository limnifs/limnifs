//! Compression tournament config manifest section.
//!
//! Records which codecs the writer tried in the compression
//! tournament and the minimum size threshold.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::cursor::ManifestCursor;
use crate::error::CoreError;

pub const COMPRESSION_TOURNAMENT_SECTION_VERSION: u8 = 1;
const MAX_CODEC_COUNT: u8 = 32;
const FLAG_SKIP_FOR_BINARY: u8 = 0x01;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressionTournamentConfig {
    pub version: u8,
    pub codecs: Vec<u8>,
    pub min_size_threshold: u32,
    pub skip_for_binary: bool,
}

pub fn parse_compression_tournament_config(
    cursor: &mut ManifestCursor<'_>,
) -> Result<CompressionTournamentConfig, CoreError> {
    let section_version = cursor.read_u8()?;
    if section_version != COMPRESSION_TOURNAMENT_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "compression_tournament version {section_version} (supported: {COMPRESSION_TOURNAMENT_SECTION_VERSION})"
            ),
        });
    }

    let codec_count = cursor.read_u8()?;
    if codec_count > MAX_CODEC_COUNT {
        return Err(CoreError::Corrupt {
            reason: format!("codec count {codec_count} exceeds cap {MAX_CODEC_COUNT}"),
        });
    }
    let codecs = cursor.read_n(usize::from(codec_count))?.to_vec();

    let min_size_threshold = cursor.read_u32_le()?;
    let flags = cursor.read_u8()?;
    let skip_for_binary = (flags & FLAG_SKIP_FOR_BINARY) != 0;

    Ok(CompressionTournamentConfig {
        version: section_version,
        codecs,
        min_size_threshold,
        skip_for_binary,
    })
}

pub fn encode_compression_tournament_config(
    config: &CompressionTournamentConfig,
    out: &mut Vec<u8>,
) {
    out.push(COMPRESSION_TOURNAMENT_SECTION_VERSION);
    let count = u8::try_from(config.codecs.len()).unwrap_or(MAX_CODEC_COUNT);
    out.push(count);
    out.extend_from_slice(&config.codecs);
    out.extend_from_slice(&config.min_size_threshold.to_le_bytes());
    let mut flags = 0u8;
    if config.skip_for_binary {
        flags |= FLAG_SKIP_FOR_BINARY;
    }
    out.push(flags);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let original = CompressionTournamentConfig {
            version: COMPRESSION_TOURNAMENT_SECTION_VERSION,
            codecs: vec![0x00, 0x01, 0x02, 0x04],
            min_size_threshold: 256,
            skip_for_binary: true,
        };
        let mut encoded = Vec::new();
        encode_compression_tournament_config(&original, &mut encoded);

        let mut cursor = ManifestCursor::new(&encoded);
        let parsed = parse_compression_tournament_config(&mut cursor).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn round_trip_empty_codecs() {
        let original = CompressionTournamentConfig {
            version: COMPRESSION_TOURNAMENT_SECTION_VERSION,
            codecs: vec![],
            min_size_threshold: 0,
            skip_for_binary: false,
        };
        let mut encoded = Vec::new();
        encode_compression_tournament_config(&original, &mut encoded);

        let mut cursor = ManifestCursor::new(&encoded);
        let parsed = parse_compression_tournament_config(&mut cursor).expect("parse");
        assert_eq!(parsed, original);
    }
}
