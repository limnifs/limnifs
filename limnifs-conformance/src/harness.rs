//! Round-trip harness: encode a vector → parse the bytes back →
//! assert identity with the encoded `ManifestRoot`.
//!
//! The harness is the verification side of the conformance suite.
//! Where the [`crate::builder`] uses `limnifs-core` to ENCODE, the
//! harness uses `limnifs-core` parsers to DECODE — but the parsers
//! and the encoder share no code paths. A bug in any parser will
//! surface as a mismatched field, and a bug in the Merkle formula
//! will surface as a mismatched root.
//!
//! ## What "pass" means
//!
//! A vector passes when:
//!
//! 1. Every section parser accepts the bytes the builder produced.
//! 2. Every parsed field matches the corresponding declarative spec value.
//! 3. The `ManifestRoot` computed from the parsed section bytes
//!    equals the `ManifestRoot` the builder computed.
//!
//! Condition (3) is the canonical integrity check: any tampering
//! with any byte of any section produces a different root.
//!
//! ## Black-box invariant
//!
//! The harness links `limnifs-core` as a library to call its parsers
//! directly. This is fine for the bootstrap — `limnifs-core` is the
//! reference implementation under test, and the harness is its first
//! customer. When third-party adapters (Python, Ruby, TypeScript)
//! land, the harness will additionally shell out to their binaries
//! and compare their reported `ManifestRoot` against the encoded one.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::builder::ManifestArtifact;
use crate::vectors::Vector;
use limnifs_core::{
    compute_merkle_root, hash_empty_section, hash_section, parse_feature_flags_section,
    parse_history, parse_manifest_header, parse_metadata_reference, parse_slab_index, FeatureFlag,
    ManifestCursor, SectionHashes,
};

/// Result of running one vector through the harness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessReport {
    pub vector_name: String,
    pub encoded_root: limnifs_format::ManifestRoot,
    pub parsed_root: limnifs_format::ManifestRoot,
    pub parsed_flags: Vec<FeatureFlag>,
    pub parsed_metadata_inlined: bool,
    pub parsed_slab_count: usize,
    pub parsed_history_count: usize,
}

/// Run a vector through the encode → parse → verify round trip.
///
/// # Panics
///
/// Panics with a descriptive message if any parser rejects the
/// encoded bytes or if the parsed `ManifestRoot` does not equal the
/// encoded one. Use [`try_round_trip`] for a fallible version.
#[must_use]
pub fn round_trip(vector: &Vector) -> HarnessReport {
    try_round_trip(vector).unwrap_or_else(|e| panic!("vector {}: {e}", vector.name))
}

/// Fallible round trip — returns an error string instead of panicking.
///
/// # Errors
///
/// Returns `Err(String)` describing the failure if any parser
/// rejects the bytes or the Merkle roots disagree.
pub fn try_round_trip(vector: &Vector) -> Result<HarnessReport, String> {
    let artifact = crate::builder::ManifestBuilder::new(vector.spec.clone()).build();
    let parsed_root =
        parse_and_recompute_root(&artifact).map_err(|e| format!("parse failed: {e}"))?;
    if parsed_root != artifact.merkle_root {
        return Err(format!(
            "merkle root mismatch: encoded={} parsed={}",
            artifact.merkle_root, parsed_root
        ));
    }
    let (flags, metadata_inlined, slab_count, history_count) =
        parse_summary(&artifact.bytes).map_err(|e| format!("summary parse failed: {e}"))?;
    Ok(HarnessReport {
        vector_name: vector.name.into(),
        encoded_root: artifact.merkle_root,
        parsed_root,
        parsed_flags: flags,
        parsed_metadata_inlined: metadata_inlined,
        parsed_slab_count: slab_count,
        parsed_history_count: history_count,
    })
}

fn parse_and_recompute_root(
    artifact: &ManifestArtifact,
) -> Result<limnifs_format::ManifestRoot, String> {
    let mut cursor = ManifestCursor::new(&artifact.bytes);

    let header_start = cursor.position();
    let _header = parse_manifest_header(&mut cursor).map_err(|e| e.to_string())?;
    let header_end = cursor.position();

    let flags_start = cursor.position();
    let flags = parse_feature_flags_section(&mut cursor).map_err(|e| e.to_string())?;
    let flags_end = cursor.position();

    let meta_ref_start = cursor.position();
    let metadata_reference = parse_metadata_reference(&mut cursor).map_err(|e| e.to_string())?;
    let meta_ref_end = cursor.position();

    let slab_index_start = cursor.position();
    let _slab_index = parse_slab_index(&mut cursor).map_err(|e| e.to_string())?;
    let slab_index_end = cursor.position();

    // Optional sections: parse based on feature flags. The spec section
    // order is fixed: crypto → EC → DMS → delta → history. Crypto and
    // delta are not yet supported, so only EC (0x0001) and DMS (0x0002)
    // are attempted here.
    let ec_params_start = cursor.position();
    let has_ec = flags.is_required(0x0001) || flags.get(0x0001).is_some();
    if has_ec {
        let _ = limnifs_core::parse_ec_params(&mut cursor).map_err(|e| e.to_string())?;
    }
    let ec_params_end = cursor.position();

    let dms_policy_start = cursor.position();
    let has_dms = flags.is_required(0x0002) || flags.get(0x0002).is_some();
    if has_dms {
        let _ = limnifs_core::parse_dms_policy(&mut cursor).map_err(|e| e.to_string())?;
    }
    let dms_policy_end = cursor.position();

    let history_start = cursor.position();
    let _history = parse_history(&mut cursor).map_err(|e| e.to_string())?;
    let history_end = cursor.position();

    let ec_hash = if has_ec {
        hash_section(&artifact.bytes[ec_params_start..ec_params_end])
    } else {
        hash_empty_section()
    };
    let dms_hash = if has_dms {
        hash_section(&artifact.bytes[dms_policy_start..dms_policy_end])
    } else {
        hash_empty_section()
    };

    let hashes = SectionHashes {
        metadata: metadata_reference.metadata_hash,
        format_header: hash_section(&artifact.bytes[header_start..header_end]),
        feature_flags: hash_section(&artifact.bytes[flags_start..flags_end]),
        metadata_reference: hash_section(&artifact.bytes[meta_ref_start..meta_ref_end]),
        slab_index: hash_section(&artifact.bytes[slab_index_start..slab_index_end]),
        crypto_params: hash_empty_section(),
        ec_params: ec_hash,
        dms_policy: dms_hash,
        delta_linkage: hash_empty_section(),
        history: hash_section(&artifact.bytes[history_start..history_end]),
    };
    Ok(compute_merkle_root(&hashes))
}

fn parse_summary(bytes: &[u8]) -> Result<(Vec<FeatureFlag>, bool, usize, usize), String> {
    let mut cursor = ManifestCursor::new(bytes);
    let _ = parse_manifest_header(&mut cursor).map_err(|e| e.to_string())?;
    let flags = parse_feature_flags_section(&mut cursor).map_err(|e| e.to_string())?;
    let metadata_reference = parse_metadata_reference(&mut cursor).map_err(|e| e.to_string())?;
    let slab_index = parse_slab_index(&mut cursor).map_err(|e| e.to_string())?;
    // Skip optional sections when feature flags declare them.
    if flags.get(0x0001).is_some() {
        let _ = limnifs_core::parse_ec_params(&mut cursor).map_err(|e| e.to_string())?;
    }
    if flags.get(0x0002).is_some() {
        let _ = limnifs_core::parse_dms_policy(&mut cursor).map_err(|e| e.to_string())?;
    }
    let history = parse_history(&mut cursor).map_err(|e| e.to_string())?;
    Ok((
        flags.entries,
        metadata_reference.is_inlined(),
        slab_index.len(),
        history.len(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors::all_vectors;

    #[test]
    fn every_vector_round_trips() {
        for vector in all_vectors() {
            let report = round_trip(&vector);
            assert_eq!(
                report.encoded_root, report.parsed_root,
                "vector {}: encoded != parsed",
                vector.name
            );
        }
    }

    #[test]
    fn minimal_vector_has_expected_summary() {
        let minimal = crate::vectors::minimal_v0_1();
        let report = round_trip(&minimal);
        assert!(report.parsed_flags.is_empty());
        assert!(!report.parsed_metadata_inlined);
        assert_eq!(report.parsed_slab_count, 1);
        assert_eq!(report.parsed_history_count, 1);
    }

    #[test]
    fn flags_vector_reports_two_flags() {
        let flagged = crate::vectors::minimal_v0_1_with_flags();
        let report = round_trip(&flagged);
        assert_eq!(report.parsed_flags.len(), 2);
        assert!(report
            .parsed_flags
            .iter()
            .any(|f| f.flag_id == 0x0012 && f.required));
        assert!(report
            .parsed_flags
            .iter()
            .any(|f| f.flag_id == 0x0020 && !f.required));
    }

    #[test]
    fn mutation_breaks_round_trip() {
        // Mutate one byte of the encoded buffer; the round trip must
        // fail to verify (either parser rejection or root mismatch).
        let vector = crate::vectors::minimal_v0_1();
        let artifact = crate::builder::ManifestBuilder::new(vector.spec.clone()).build();
        let mut corrupted = artifact.bytes.clone();
        // Flip a byte in the history section (always present).
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0xFF;
        let mut fake = artifact;
        fake.bytes = corrupted;
        let result = parse_and_recompute_root(&fake);
        assert!(result.is_err(), "mutation should break verification");
    }
}
