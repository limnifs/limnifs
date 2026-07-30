//! DMS policy section (spec §5.7, `bit-level/44-dms-policy.md`).
//!
//! Carries a Dead Man's Switch / key escrow record. v0.1 supports
//! Shamir k-of-n secret sharing only — time-lock puzzles are deferred
//! (§21.2). The section is OPTIONAL — present iff the DMS feature
//! flag (`0x0002`) is declared in the feature flags section (§5.2).

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::cursor::ManifestCursor;
use crate::error::CoreError;

/// Current layout version of this section.
pub const DMS_POLICY_SECTION_VERSION: u8 = 1;

/// Scheme selector: Shamir k-of-n secret sharing (the only scheme
/// supported in v0.1).
pub const DMS_SCHEME_SHAMIR: u8 = 0x00;

/// Sentinel value for `scheme` indicating an extended (post-v1)
/// scheme descriptor follows. Readers in v1 reject.
pub const DMS_SCHEME_EXTENDED: u8 = 0xFF;

/// Maximum total share count (Shamir-over-GF(256) limit).
pub const MAX_TOTAL_SHARES: u8 = 255;

/// Default per-share data ceiling. Shamir share sizes are bounded by
/// the secret length (typically 32 bytes for an image key).
pub const DEFAULT_SHARE_DATA_MAX_BYTES: u32 = 1024;

/// Default ceiling on the `reconstruction_hint` length.
pub const DEFAULT_HINT_MAX_BYTES: u32 = 4 * 1024;

/// Width of the fixed prefix of this section.
const PREFIX_LEN: usize = 1 + 1 + 1 + 1 + 4;

/// One share: a custodian identifier plus the share bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShareRecord {
    pub custodian_id: String,
    pub share_data: Vec<u8>,
}

/// Parsed DMS policy section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DmsPolicy {
    pub k: u8,
    pub n: u8,
    pub shares: Vec<ShareRecord>,
    pub reconstruction_hint: Option<String>,
}

/// Parse the DMS policy section from the cursor's current position.
///
/// Uses the default ceilings (1 KiB per `share_data`, 4 KiB hint).
///
/// # Errors
///
/// - [`CoreError::UnsupportedFeature`] if `section_version` is not 1
///   or `scheme` is not Shamir (`0x00`).
/// - [`CoreError::Corrupt`] if the `k`/`n` constraints are violated
///   (`1 ≤ k ≤ n ≤ 255`), `share_count != n`, a `custodian_id` is
///   empty or duplicated, a `share_data` violates its ceiling, or
///   the `reconstruction_hint` bytes are not valid UTF-8.
/// - [`CoreError::TooShort`] if the cursor underruns.
pub fn parse_dms_policy(cursor: &mut ManifestCursor<'_>) -> Result<DmsPolicy, CoreError> {
    parse_dms_policy_with_ceilings(cursor, DEFAULT_SHARE_DATA_MAX_BYTES, DEFAULT_HINT_MAX_BYTES)
}

/// Same as [`parse_dms_policy`] but with caller-supplied ceilings.
///
/// # Errors
///
/// Inherits all errors from [`parse_dms_policy`].
pub fn parse_dms_policy_with_ceilings(
    cursor: &mut ManifestCursor<'_>,
    max_share_data_bytes: u32,
    max_hint_bytes: u32,
) -> Result<DmsPolicy, CoreError> {
    let section_version = cursor.read_u8()?;
    if section_version != DMS_POLICY_SECTION_VERSION {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "dms_policy section version {section_version} (supported: {DMS_POLICY_SECTION_VERSION})"
            ),
        });
    }
    let scheme = cursor.read_u8()?;
    if scheme == DMS_SCHEME_EXTENDED {
        return Err(CoreError::UnsupportedFeature {
            feature: "dms_policy scheme 0xFF (extended descriptor, post-v1)".into(),
        });
    }
    if scheme != DMS_SCHEME_SHAMIR {
        return Err(CoreError::UnsupportedFeature {
            feature: format!(
                "dms_policy scheme 0x{scheme:02X} (supported: 0x{DMS_SCHEME_SHAMIR:02X} Shamir)"
            ),
        });
    }
    let k = cursor.read_u8()?;
    let n = cursor.read_u8()?;
    validate_shamir_params(k, n)?;
    let share_count = cursor.read_u32_le()?;
    if u16::from(n) != u16::try_from(share_count).unwrap_or(0) {
        return Err(CoreError::Corrupt {
            reason: format!("dms_policy share_count {share_count} does not equal n ({n})"),
        });
    }
    let count_us = usize::try_from(share_count).map_err(|_| CoreError::Corrupt {
        reason: format!("dms_policy share_count {share_count} exceeds usize"),
    })?;
    let mut shares = Vec::with_capacity(count_us);
    let mut seen_custodians: std::collections::HashSet<String> = std::collections::HashSet::new();
    for index in 0..count_us {
        let share = parse_share_record(cursor, index, max_hint_bytes, max_share_data_bytes)?;
        if !seen_custodians.insert(share.custodian_id.clone()) {
            return Err(CoreError::Corrupt {
                reason: format!(
                    "dms_policy share {index}: duplicate custodian_id {:?}",
                    share.custodian_id
                ),
            });
        }
        shares.push(share);
    }
    let reconstruction_hint = parse_reconstruction_hint(cursor, max_hint_bytes)?;
    let _ = PREFIX_LEN; // documented constant; nothing to emit
    Ok(DmsPolicy {
        k,
        n,
        shares,
        reconstruction_hint,
    })
}

fn parse_share_record(
    cursor: &mut ManifestCursor<'_>,
    index: usize,
    max_hint_bytes: u32,
    max_share_data_bytes: u32,
) -> Result<ShareRecord, CoreError> {
    let custodian_id_len = cursor.read_u32_le()?;
    let cid = read_string_with_ceiling(cursor, custodian_id_len, max_hint_bytes, "custodian_id")?;
    if cid.is_empty() {
        return Err(CoreError::Corrupt {
            reason: format!("dms_policy share {index}: empty custodian_id"),
        });
    }
    let share_data_len = cursor.read_u32_le()?;
    if share_data_len == 0 {
        return Err(CoreError::Corrupt {
            reason: format!("dms_policy share {index}: zero-length share_data"),
        });
    }
    if share_data_len > max_share_data_bytes {
        return Err(CoreError::Corrupt {
            reason: format!(
                "dms_policy share {index}: share_data_len {share_data_len} exceeds ceiling {max_share_data_bytes}"
            ),
        });
    }
    let share_data = cursor.read_n_owned(usize::try_from(share_data_len).map_err(|_| {
        CoreError::Corrupt {
            reason: format!("dms_policy share_data_len {share_data_len} exceeds usize"),
        }
    })?)?;
    Ok(ShareRecord {
        custodian_id: cid,
        share_data,
    })
}

fn parse_reconstruction_hint(
    cursor: &mut ManifestCursor<'_>,
    max_hint_bytes: u32,
) -> Result<Option<String>, CoreError> {
    let hint_len = cursor.read_u32_le()?;
    if hint_len == 0 {
        return Ok(None);
    }
    if hint_len > max_hint_bytes {
        return Err(CoreError::Corrupt {
            reason: format!(
                "dms_policy reconstruction_hint_len {hint_len} exceeds ceiling {max_hint_bytes}"
            ),
        });
    }
    let bytes = cursor.read_n(usize::try_from(hint_len).map_err(|_| CoreError::Corrupt {
        reason: format!("dms_policy hint_len {hint_len} exceeds usize"),
    })?)?;
    let s = std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| CoreError::Corrupt {
            reason: "dms_policy reconstruction_hint is not valid UTF-8".into(),
        })?;
    Ok(Some(s))
}

fn validate_shamir_params(k: u8, n: u8) -> Result<(), CoreError> {
    if k == 0 {
        return Err(CoreError::Corrupt {
            reason: "dms_policy k must be >= 1, got 0".into(),
        });
    }
    if n == 0 {
        return Err(CoreError::Corrupt {
            reason: "dms_policy n must be >= 1, got 0".into(),
        });
    }
    if k > n {
        return Err(CoreError::Corrupt {
            reason: format!("dms_policy k ({k}) must be <= n ({n})"),
        });
    }
    // n is u8 so always <= MAX_TOTAL_SHARES (=255 = u8::MAX); the
    // constant is kept for documentation and to pin the spec
    // invariant for future scheme variants that may widen the type.
    Ok(())
}

fn read_string_with_ceiling(
    cursor: &mut ManifestCursor<'_>,
    raw_len: u32,
    ceiling: u32,
    label: &str,
) -> Result<String, CoreError> {
    if raw_len > ceiling {
        return Err(CoreError::Corrupt {
            reason: format!("dms_policy {label} length {raw_len} exceeds ceiling {ceiling}"),
        });
    }
    let len = usize::try_from(raw_len).map_err(|_| CoreError::Corrupt {
        reason: format!("dms_policy {label} length {raw_len} exceeds usize"),
    })?;
    let bytes = cursor.read_n(len)?;
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| CoreError::Corrupt {
            reason: format!("dms_policy {label} is not valid UTF-8"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_share_bytes(custodian_id: &str, share_data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let cid_len = u32::try_from(custodian_id.len()).expect("cid len fits u32");
        bytes.extend_from_slice(&cid_len.to_le_bytes());
        bytes.extend_from_slice(custodian_id.as_bytes());
        let sd_len = u32::try_from(share_data.len()).expect("sd len fits u32");
        bytes.extend_from_slice(&sd_len.to_le_bytes());
        bytes.extend_from_slice(share_data);
        bytes
    }

    #[allow(clippy::vec_init_then_push)]
    fn make_dms_bytes(
        version: u8,
        scheme: u8,
        k: u8,
        n: u8,
        shares: &[(&str, Vec<u8>)],
        hint: Option<&str>,
    ) -> Vec<u8> {
        let mut bytes = vec![version, scheme, k, n];
        let count = u32::try_from(shares.len()).expect("count fits u32");
        bytes.extend_from_slice(&count.to_le_bytes());
        for (cid, sd) in shares {
            bytes.extend(make_share_bytes(cid, sd));
        }
        match hint {
            None => bytes.extend_from_slice(&0u32.to_le_bytes()),
            Some(h) => {
                let h_bytes = h.as_bytes();
                let h_len = u32::try_from(h_bytes.len()).expect("hint len fits u32");
                bytes.extend_from_slice(&h_len.to_le_bytes());
                bytes.extend_from_slice(h_bytes);
            }
        }
        bytes
    }

    fn sample_share(byte: u8) -> Vec<u8> {
        vec![byte; 32]
    }

    #[test]
    fn parses_3_of_5_no_hint() {
        let bytes = make_dms_bytes(
            DMS_POLICY_SECTION_VERSION,
            DMS_SCHEME_SHAMIR,
            3,
            5,
            &[
                ("alice1", sample_share(0xAA)),
                ("bob123", sample_share(0xBB)),
                ("carol4", sample_share(0xCC)),
                ("dave99", sample_share(0xDD)),
                ("eve555", sample_share(0xEE)),
            ],
            None,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_dms_policy(&mut cursor).expect("3-of-5 parses");
        assert_eq!(parsed.k, 3);
        assert_eq!(parsed.n, 5);
        assert_eq!(parsed.shares.len(), 5);
        assert!(parsed.reconstruction_hint.is_none());
        assert_eq!(cursor.position(), bytes.len());
    }

    #[test]
    fn parses_2_of_3_with_hint() {
        let hint = "Contact legal@example.com to coordinate share assembly.";
        let bytes = make_dms_bytes(
            DMS_POLICY_SECTION_VERSION,
            DMS_SCHEME_SHAMIR,
            2,
            3,
            &[
                ("ceo", sample_share(0x11)),
                ("cfo", sample_share(0x22)),
                ("coo", sample_share(0x33)),
            ],
            Some(hint),
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_dms_policy(&mut cursor).expect("2-of-3 parses");
        assert_eq!(parsed.k, 2);
        assert_eq!(parsed.n, 3);
        assert_eq!(parsed.reconstruction_hint.as_deref(), Some(hint));
    }

    #[test]
    fn parses_1_of_1_unanimous_single_share() {
        let bytes = make_dms_bytes(
            DMS_POLICY_SECTION_VERSION,
            DMS_SCHEME_SHAMIR,
            1,
            1,
            &[("solo", sample_share(0xFF))],
            None,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        let parsed = parse_dms_policy(&mut cursor).expect("1-of-1 parses");
        assert_eq!(parsed.k, 1);
        assert_eq!(parsed.n, 1);
    }

    #[test]
    fn rejects_unknown_section_version() {
        let bytes = make_dms_bytes(
            7,
            DMS_SCHEME_SHAMIR,
            2,
            2,
            &[("a", sample_share(0)), ("b", sample_share(1))],
            None,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_dms_policy(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("version 7"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unsupported_scheme() {
        let bytes = make_dms_bytes(
            DMS_POLICY_SECTION_VERSION,
            0x05,
            2,
            2,
            &[("a", sample_share(0)), ("b", sample_share(1))],
            None,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_dms_policy(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("scheme 0x05"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_extended_scheme_sentinel() {
        let bytes = make_dms_bytes(
            DMS_POLICY_SECTION_VERSION,
            DMS_SCHEME_EXTENDED,
            2,
            2,
            &[("a", sample_share(0)), ("b", sample_share(1))],
            None,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_dms_policy(&mut cursor) {
            Err(CoreError::UnsupportedFeature { feature }) => {
                assert!(feature.contains("0xFF"), "got: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[test]
    fn rejects_zero_k() {
        let bytes = make_dms_bytes(
            DMS_POLICY_SECTION_VERSION,
            DMS_SCHEME_SHAMIR,
            0,
            2,
            &[("a", sample_share(0)), ("b", sample_share(1))],
            None,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_dms_policy(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("k must be >= 1"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_k_greater_than_n() {
        let bytes = make_dms_bytes(
            DMS_POLICY_SECTION_VERSION,
            DMS_SCHEME_SHAMIR,
            5,
            3,
            &[
                ("a", sample_share(0)),
                ("b", sample_share(1)),
                ("c", sample_share(2)),
            ],
            None,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_dms_policy(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("k (5) must be <= n (3)"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_share_count_not_equal_n() {
        let bytes = vec![DMS_POLICY_SECTION_VERSION, DMS_SCHEME_SHAMIR, 2, 3];
        let bytes = [bytes.as_slice(), &99u32.to_le_bytes()].concat();
        // share_count = 99 != n = 3. Parser should reject at the check.
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_dms_policy(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("share_count"), "got: {reason}");
                assert!(reason.contains("99"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_duplicate_custodian_id() {
        let bytes = make_dms_bytes(
            DMS_POLICY_SECTION_VERSION,
            DMS_SCHEME_SHAMIR,
            2,
            2,
            &[("alice", sample_share(0)), ("alice", sample_share(1))],
            None,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_dms_policy(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("duplicate"), "got: {reason}");
                assert!(reason.contains("alice"));
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_custodian_id() {
        let bytes = make_dms_bytes(
            DMS_POLICY_SECTION_VERSION,
            DMS_SCHEME_SHAMIR,
            1,
            1,
            &[("", sample_share(0))],
            None,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_dms_policy(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("empty custodian_id"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_zero_share_data() {
        let bytes = make_dms_bytes(
            DMS_POLICY_SECTION_VERSION,
            DMS_SCHEME_SHAMIR,
            1,
            1,
            &[("alice", Vec::new())],
            None,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_dms_policy(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("zero-length share_data"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_share_data_above_ceiling() {
        let oversized = vec![0xFF; (DEFAULT_SHARE_DATA_MAX_BYTES as usize) + 1];
        let bytes = make_dms_bytes(
            DMS_POLICY_SECTION_VERSION,
            DMS_SCHEME_SHAMIR,
            1,
            1,
            &[("alice", oversized)],
            None,
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_dms_policy(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("exceeds ceiling"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_hint_above_ceiling() {
        let oversized_hint = "x".repeat((DEFAULT_HINT_MAX_BYTES as usize) + 1);
        let bytes = make_dms_bytes(
            DMS_POLICY_SECTION_VERSION,
            DMS_SCHEME_SHAMIR,
            1,
            1,
            &[("alice", sample_share(0))],
            Some(&oversized_hint),
        );
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_dms_policy(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("reconstruction_hint_len"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn rejects_non_utf8_hint() {
        let mut bytes = make_dms_bytes(
            DMS_POLICY_SECTION_VERSION,
            DMS_SCHEME_SHAMIR,
            1,
            1,
            &[("alice", sample_share(0))],
            None,
        );
        // Replace the trailing hint_len=0 + (nothing) with hint_len=2 + invalid UTF-8.
        let last4 = bytes.len() - 4;
        bytes[last4..last4 + 4].copy_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&[0xFF, 0xFE]);
        let mut cursor = ManifestCursor::new(&bytes);
        match parse_dms_policy(&mut cursor) {
            Err(CoreError::Corrupt { reason }) => {
                assert!(reason.contains("UTF-8"), "got: {reason}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn max_total_shares_constant_is_gf_256_limit() {
        // n is u8 so the parser-level check `n > MAX_TOTAL_SHARES`
        // is unreachable; the constant pins the spec invariant.
        assert_eq!(MAX_TOTAL_SHARES, 255);
    }
}
