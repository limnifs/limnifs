//! Categorization policy manifest section.
//!
//! Records the file categorizer rules used at write time so the
//! reader can reconstruct the writer's codec routing decisions.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::cursor::ManifestCursor;
use crate::error::CoreError;

pub const CATEGORIZATION_POLICY_SECTION_VERSION: u8 = 1;
const MAX_RULE_COUNT: u32 = 1024;
const MAX_NAME_LEN: usize = 64;
const MAX_EXT_LEN: usize = 16;
const MAX_EXT_COUNT: u32 = 64;
const MAX_MAGIC_LEN: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategorizationPolicy {
    pub version: u8,
    pub rules: Vec<CategorizerRule>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategorizerRule {
    pub name: String,
    pub extensions: Vec<String>,
    pub magic_bytes: Vec<u8>,
    pub codec: u8,
    pub max_size: Option<u32>,
    pub enabled: bool,
}

pub fn parse_categorization_policy(
    cursor: &mut ManifestCursor<'_>,
) -> Result<CategorizationPolicy, CoreError> {
    let section_version = cursor.read_u8()?;
    if section_version != CATEGORIZATION_POLICY_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "categorization_policy version {section_version} (supported: {CATEGORIZATION_POLICY_SECTION_VERSION})"
            ),
        });
    }

    let raw_count = cursor.read_u32_le()?;
    let count = usize::try_from(raw_count).map_err(|_| CoreError::Corrupt {
        reason: format!("categorization_policy rule count {raw_count} exceeds usize"),
    })?;
    if raw_count > MAX_RULE_COUNT {
        return Err(CoreError::Corrupt {
            reason: format!(
                "categorization_policy rule count {raw_count} exceeds cap {MAX_RULE_COUNT}"
            ),
        });
    }

    let mut rules = Vec::with_capacity(count);
    for _ in 0..count {
        rules.push(parse_rule(cursor)?);
    }

    Ok(CategorizationPolicy {
        version: section_version,
        rules,
    })
}

fn parse_rule(cursor: &mut ManifestCursor<'_>) -> Result<CategorizerRule, CoreError> {
    let name_len = usize::from(cursor.read_u8()?);
    if name_len > MAX_NAME_LEN {
        return Err(CoreError::Corrupt {
            reason: format!("categorizer name length {name_len} exceeds cap {MAX_NAME_LEN}"),
        });
    }
    let name =
        String::from_utf8(cursor.read_n(name_len)?.to_vec()).map_err(|_| CoreError::Corrupt {
            reason: "categorizer name is not valid UTF-8".into(),
        })?;

    let ext_count = cursor.read_u32_le()?;
    if ext_count > MAX_EXT_COUNT {
        return Err(CoreError::Corrupt {
            reason: format!("extension count {ext_count} exceeds cap {MAX_EXT_COUNT}"),
        });
    }
    let mut extensions = Vec::with_capacity(usize::try_from(ext_count).unwrap_or(0));
    for _ in 0..ext_count {
        let ext_len = usize::from(cursor.read_u8()?);
        if ext_len > MAX_EXT_LEN {
            return Err(CoreError::Corrupt {
                reason: format!("extension length {ext_len} exceeds cap {MAX_EXT_LEN}"),
            });
        }
        let ext = String::from_utf8(cursor.read_n(ext_len)?.to_vec()).map_err(|_| {
            CoreError::Corrupt {
                reason: "extension is not valid UTF-8".into(),
            }
        })?;
        extensions.push(ext);
    }

    let magic_len = usize::from(cursor.read_u16_le()?);
    if magic_len > MAX_MAGIC_LEN {
        return Err(CoreError::Corrupt {
            reason: format!("magic length {magic_len} exceeds cap {MAX_MAGIC_LEN}"),
        });
    }
    let magic_bytes = cursor.read_n(magic_len)?.to_vec();

    let codec = cursor.read_u8()?;
    let flags = cursor.read_u8()?;
    let enabled = (flags & 0x01) != 0;
    let has_max_size = (flags & 0x02) != 0;

    let max_size = if has_max_size {
        Some(cursor.read_u32_le()?)
    } else {
        None
    };

    Ok(CategorizerRule {
        name,
        extensions,
        magic_bytes,
        codec,
        max_size,
        enabled,
    })
}

pub fn encode_categorization_policy(policy: &CategorizationPolicy, out: &mut Vec<u8>) {
    out.push(CATEGORIZATION_POLICY_SECTION_VERSION);
    let count = u32::try_from(policy.rules.len()).unwrap_or(MAX_RULE_COUNT);
    out.extend_from_slice(&count.to_le_bytes());

    for rule in &policy.rules {
        encode_rule(rule, out);
    }
}

fn encode_rule(rule: &CategorizerRule, out: &mut Vec<u8>) {
    let name_bytes = rule.name.as_bytes();
    let name_len = u8::try_from(name_bytes.len()).unwrap_or(u8::MAX);
    out.push(name_len);
    out.extend_from_slice(&name_bytes[..name_len as usize]);

    let ext_count = u32::try_from(rule.extensions.len()).unwrap_or(0);
    out.extend_from_slice(&ext_count.to_le_bytes());
    for ext in &rule.extensions {
        let ext_bytes = ext.as_bytes();
        let ext_len = u8::try_from(ext_bytes.len()).unwrap_or(u8::MAX);
        out.push(ext_len);
        out.extend_from_slice(&ext_bytes[..ext_len as usize]);
    }

    let magic_len = u16::try_from(rule.magic_bytes.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&magic_len.to_le_bytes());
    out.extend_from_slice(&rule.magic_bytes);

    out.push(rule.codec);

    let mut flags = 0u8;
    if rule.enabled {
        flags |= 0x01;
    }
    if rule.max_size.is_some() {
        flags |= 0x02;
    }
    out.push(flags);

    if let Some(max_size) = rule.max_size {
        out.extend_from_slice(&max_size.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_policy() -> CategorizationPolicy {
        CategorizationPolicy {
            version: CATEGORIZATION_POLICY_SECTION_VERSION,
            rules: vec![
                CategorizerRule {
                    name: "dna".into(),
                    extensions: vec!["fasta".into(), "fa".into()],
                    magic_bytes: vec![b'N', b'C', b'B', b'I'],
                    codec: 0x0D,
                    max_size: Some(524_288),
                    enabled: true,
                },
                CategorizerRule {
                    name: "json".into(),
                    extensions: vec!["json".into()],
                    magic_bytes: vec![],
                    codec: 0x09,
                    max_size: None,
                    enabled: true,
                },
            ],
        }
    }

    #[test]
    fn round_trip() {
        let original = sample_policy();
        let mut encoded = Vec::new();
        encode_categorization_policy(&original, &mut encoded);

        let mut cursor = ManifestCursor::new(&encoded);
        let parsed = parse_categorization_policy(&mut cursor).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn round_trip_empty() {
        let original = CategorizationPolicy {
            version: CATEGORIZATION_POLICY_SECTION_VERSION,
            rules: vec![],
        };
        let mut encoded = Vec::new();
        encode_categorization_policy(&original, &mut encoded);

        let mut cursor = ManifestCursor::new(&encoded);
        let parsed = parse_categorization_policy(&mut cursor).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn rejects_bad_version() {
        let mut encoded = vec![0xFF, 0x00, 0x00, 0x00, 0x00];
        let mut cursor = ManifestCursor::new(&encoded);
        assert!(parse_categorization_policy(&mut cursor).is_err());
    }
}
