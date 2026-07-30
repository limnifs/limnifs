//! EC params section (spec §5.6, `bit-level/43-ec-params.md`).
//!
//! Configures Reed-Solomon erasure coding for the image's slabs. The
//! section is OPTIONAL — present iff the EC feature flag (`0x0001`)
//! is declared in the feature flags section (§5.2).

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::cursor::ManifestCursor;
use crate::error::CoreError;
use limnifs_format::SlabId;

/// Current layout version of this section.
pub const EC_PARAMS_SECTION_VERSION: u8 = 1;

/// Default GF(2^8) generator polynomial (the canonical AES polynomial).
pub const DEFAULT_EC_POLYNOMIAL: u16 = 0x011D;

/// GF(2^8) shard count upper bound.
pub const MAX_SHARDS: u8 = 255;

/// Width of the fixed prefix of this section (`version` + `k` + `m` + `polynomial` + `override_count`).
const PREFIX_LEN: usize = 1 + 1 + 1 + 2 + 4;

/// Width of a single override entry (40-byte `SlabId` + `k` + `m`).
const OVERRIDE_ENTRY_LEN: usize = 40 + 1 + 1;

/// One per-slab (k, m) override.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EcOverride {
    pub slab_id: SlabId,
    pub k: u8,
    pub m: u8,
}

/// Parsed EC params section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EcParams {
    pub k: u8,
    pub m: u8,
    pub polynomial: u16,
    pub overrides: Vec<EcOverride>,
}

impl EcParams {
    #[must_use]
    pub fn default_params(k: u8, m: u8) -> Self {
        Self {
            k,
            m,
            polynomial: DEFAULT_EC_POLYNOMIAL,
            overrides: Vec::new(),
        }
    }
}

/// Parse the EC params section from the cursor's current position.
///
/// # Errors
///
/// - [`CoreError::UnsupportedFeature`] if `section_version` is not 1
///   or `polynomial` is not the supported value.
/// - [`CoreError::Corrupt`] if `k < 1`, `m < 1`, `k + m > 255`, if
///   any override's `(k, m)` violates the same constraints, or if a
///   `slab_id` appears in multiple overrides.
/// - [`CoreError::TooShort`] if the cursor has fewer bytes than the
///   declared structure requires.
pub fn parse_ec_params(cursor: &mut ManifestCursor<'_>) -> Result<EcParams, CoreError> {
    let section_version = cursor.read_u8()?;
    if section_version != EC_PARAMS_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "ec_params section version {section_version} (supported: {EC_PARAMS_SECTION_VERSION})"
            ),
        });
    }
    let k = cursor.read_u8()?;
    let m = cursor.read_u8()?;
    validate_shard_counts(k, m, "default")?;
    let polynomial = cursor.read_u16_le()?;
    if polynomial != DEFAULT_EC_POLYNOMIAL {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "ec_params polynomial 0x{polynomial:04X} (supported: 0x{DEFAULT_EC_POLYNOMIAL:04X})"
            ),
        });
    }
    let raw_count = cursor.read_u32_le()?;
    let override_count = usize::try_from(raw_count).map_err(|_| CoreError::Corrupt {
        reason: format!("ec_params override_count {raw_count} exceeds usize"),
    })?;
    // DoS check: each override needs at least OVERRIDE_ENTRY_LEN bytes.
    let min_size = override_count
        .checked_mul(OVERRIDE_ENTRY_LEN)
        .ok_or_else(|| CoreError::Corrupt {
            reason: format!("ec_params override_count {override_count} overflows usize"),
        })?;
    if cursor.remaining_len() < min_size {
        return Err(CoreError::TooShort {
            have: cursor.remaining_len(),
            need: min_size,
        });
    }
    let mut overrides = Vec::with_capacity(override_count);
    for index in 0..override_count {
        let ordinal = cursor.read_u64_le()?;
        let hash_bytes = cursor.read_n(32)?;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(hash_bytes);
        let slab_id = SlabId::new(ordinal, hash);
        let ok = cursor.read_u8()?;
        let om = cursor.read_u8()?;
        validate_shard_counts(
            ok,
            om,
            format!("override {index} for slab ordinal {ordinal}").as_str(),
        )?;
        if overrides
            .iter()
            .any(|existing: &EcOverride| existing.slab_id == slab_id)
        {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "ec_params override {index}: duplicate slab_id (ordinal {ordinal})"
                ),
            });
        }
        overrides.push(EcOverride {
            slab_id,
            k: ok,
            m: om,
        });
    }
    let _ = PREFIX_LEN; // documented constant; nothing to emit
    Ok(EcParams {
        k,
        m,
        polynomial,
        overrides,
    })
}

fn validate_shard_counts(k: u8, m: u8, label: &str) -> Result<(), CoreError> {
    if k == 0 {
        return Err(CoreError::Corrupt {
            reason: format!("ec_params {label}: k must be >= 1, got 0"),
        });
    }
    if m == 0 {
        return Err(CoreError::Corrupt {
            reason: format!("ec_params {label}: m must be >= 1, got 0"),
        });
    }
    let total = u16::from(k) + u16::from(m);
    if total > u16::from(MAX_SHARDS) {
        return Err(CoreError::Corrupt {
            reason: format!(
                "ec_params {label}: k + m = {total} exceeds GF(2^8) limit ({MAX_SHARDS})"
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ec_params_bytes(
        version: u8,
        k: u8,
        m: u8,
        polynomial: u16,
        overrides: &[(u64, [u8; 32], u8, u8)],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(version);
        bytes.push(k);
        bytes.push(m);
        bytes.extend_from_slice(&polynomial.to_le_bytes());
        let count = u32::try_from(overrides.len()).expect("count fits u32");
        bytes.extend_from_slice(&count.to_le_bytes());
        for (ordinal, hash, ok, om) in overrides {
            bytes.extend_from_slice(&ordinal.to_le_bytes());
            bytes.extend_from_slice(hash);
            bytes.push(*ok);
            bytes.push(*om);
        }
        bytes
    }

    fn sample_hash(byte: u8) -> [u8; 32] {
        let mut h = [0u8; 32];
        h[0] = byte;
        h
    }

    #[test]
    fn parses_default_only() {
        let bytes =
            make_ec_params_bytes(EC_PARAMS_SECTION_VERSION, 4, 2, DEFAULT_EC_POLYNOMIAL, &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_ec_params(&mut cursor).expect("default parses");
        assert_eq!(parsed.k, 4);
        assert_eq!(parsed.m, 2);
        assert_eq!(parsed.polynomial, DEFAULT_EC_POLYNOMIAL);
        assert!(parsed.overrides.is_empty());
        assert_eq!(cursor.position(), bytes.len());
    }

    #[test]
    fn parses_with_one_override() {
        let bytes = make_ec_params_bytes(
            EC_PARAMS_SECTION_VERSION,
            4,
            2,
            DEFAULT_EC_POLYNOMIAL,
            &[(7, sample_hash(0xAA), 8, 4)],
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_ec_params(&mut cursor).expect("override parses");
        assert_eq!(parsed.overrides.len(), 1);
        assert_eq!(parsed.overrides[0].slab_id.ordinal, 7);
        assert_eq!(parsed.overrides[0].k, 8);
        assert_eq!(parsed.overrides[0].m, 4);
    }

    #[test]
    fn parses_with_multiple_overrides() {
        let bytes = make_ec_params_bytes(
            EC_PARAMS_SECTION_VERSION,
            4,
            2,
            DEFAULT_EC_POLYNOMIAL,
            &[
                (0, sample_hash(0x01), 6, 3),
                (1, sample_hash(0x02), 8, 4),
                (2, sample_hash(0x03), 16, 8),
            ],
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_ec_params(&mut cursor).expect("multi parses");
        assert_eq!(parsed.overrides.len(), 3);
    }

    #[test]
    fn rejects_unknown_section_version() {
        let bytes = make_ec_params_bytes(7, 4, 2, DEFAULT_EC_POLYNOMIAL, &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_ec_params(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("version 7"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_zero_k() {
        let bytes =
            make_ec_params_bytes(EC_PARAMS_SECTION_VERSION, 0, 2, DEFAULT_EC_POLYNOMIAL, &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_ec_params(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("k must be >= 1"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_zero_m() {
        let bytes =
            make_ec_params_bytes(EC_PARAMS_SECTION_VERSION, 4, 0, DEFAULT_EC_POLYNOMIAL, &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_ec_params(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("m must be >= 1"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_shard_count_above_gf_limit() {
        // k = 200, m = 100 -> total 300 > 255.
        let bytes = make_ec_params_bytes(
            EC_PARAMS_SECTION_VERSION,
            200,
            100,
            DEFAULT_EC_POLYNOMIAL,
            &[],
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_ec_params(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("GF(2^8)"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_polynomial() {
        let bytes = make_ec_params_bytes(EC_PARAMS_SECTION_VERSION, 4, 2, 0x002B, &[]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_ec_params(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("polynomial"), "got: {feature}");
                assert!(feature.contains("0x002B"));
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_duplicate_override_slab_id() {
        let bytes = make_ec_params_bytes(
            EC_PARAMS_SECTION_VERSION,
            4,
            2,
            DEFAULT_EC_POLYNOMIAL,
            &[(5, sample_hash(0xAA), 6, 3), (5, sample_hash(0xAA), 8, 4)],
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_ec_params(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("duplicate"), "got: {reason}");
                assert!(reason.contains("ordinal 5"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_override_with_bad_shard_counts() {
        let bytes = make_ec_params_bytes(
            EC_PARAMS_SECTION_VERSION,
            4,
            2,
            DEFAULT_EC_POLYNOMIAL,
            &[(7, sample_hash(0xAA), 0, 4)], // override k = 0
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_ec_params(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("override 0"), "got: {reason}");
                assert!(reason.contains("k must be >= 1"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_truncated_prefix() {
        // Section version + partial k.
        let bytes = [EC_PARAMS_SECTION_VERSION];
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_ec_params(&mut cursor) {
            Err(CoreError::TooShort { .. }) => {}
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn rejects_override_count_overrunning_buffer() {
        let mut bytes = Vec::new();
        bytes.push(EC_PARAMS_SECTION_VERSION);
        bytes.push(4);
        bytes.push(2);
        bytes.extend_from_slice(&DEFAULT_EC_POLYNOMIAL.to_le_bytes());
        bytes.extend_from_slice(&10u32.to_le_bytes()); // claim 10 overrides, none provided
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_ec_params(&mut cursor) {
            Err(CoreError::TooShort { have, need }) => {
                assert_eq!(have, 0);
                assert_eq!(need, 10 * OVERRIDE_ENTRY_LEN);
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn default_params_helper_uses_default_polynomial() {
        let params = EcParams::default_params(4, 2);
        assert_eq!(params.polynomial, DEFAULT_EC_POLYNOMIAL);
        assert!(params.overrides.is_empty());
    }

    #[test]
    fn max_shards_constant_is_255() {
        // GF(2^8) has 256 elements; one is the zero symbol, so 255 usable shards.
        assert_eq!(MAX_SHARDS, 255);
    }
}
