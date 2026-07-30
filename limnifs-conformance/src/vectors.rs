//! Declarative test vectors.
//!
//! Each vector is a [`crate::ManifestSpec`] plus a human-readable
//! name and description. Vectors are declarative — the
//! [`crate::builder`] encodes them into wire bytes; the
//! [`crate::harness`] parses those bytes back and asserts identity
//! with the vector's computed `ManifestRoot`.
//!
//! Add new vectors by appending here. The harness auto-discovers
//! every vector returned by [`all_vectors`].

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::builder::{
    FeatureFlagSpec, HeaderSpec, HistoryEntrySpec, HistoryOpSpec, ManifestSpec,
    MetadataReferenceSpec, SlabIndexEntrySpec,
};
use limnifs_format::SlabId;

/// A named test vector.
#[derive(Clone, Debug)]
pub struct Vector {
    /// Short identifier used in test names and CI logs. Kebab-case.
    pub name: &'static str,
    /// Human-readable description of what the vector exercises.
    pub description: &'static str,
    /// The declarative manifest spec.
    pub spec: ManifestSpec,
}

/// Smallest valid v0.1 plaintext non-delta image.
///
/// - Header: all three layers at version 1.
/// - Feature flags: empty.
/// - Metadata reference: external, single `file:` locator.
/// - Slab index: single slab, single `file:` locator.
/// - History: single `build` entry with timestamp 0 (deterministic
///   mode per §1.4).
///
/// No optional sections (crypto/EC/DMS/delta). This is the
/// minimum-viable image that any conformant v0.1 reader MUST parse
/// and verify.
#[must_use]
pub fn minimal_v0_1() -> Vector {
    Vector {
        name: "minimal-v0-1",
        description: "Smallest valid v0.1 plaintext non-delta image",
        spec: ManifestSpec {
            header: HeaderSpec::current(),
            feature_flags: Vec::new(),
            metadata_reference: MetadataReferenceSpec::External {
                metadata_hash: [0xAA; 32],
                locators: vec!["file:///var/lib/limnifs/metadata.bin".into()],
            },
            slab_index: vec![SlabIndexEntrySpec {
                slab_id: SlabId::new(0, [0u8; 32]),
                locators: vec!["file:///var/lib/limnifs/slab-0.bin".into()],
            }],
            history: vec![HistoryEntrySpec {
                op: HistoryOpSpec::Build,
                timestamp_ns: 0,
                inputs: Vec::new(),
                params: Vec::new(),
            }],
        },
    }
}

/// Like [`minimal_v0_1`] but declares EC (required) and `https:`
/// (optional) in the feature flags. Slab index still has one entry;
/// the test asserts that flag presence doesn't change the encoding
/// rules for the required sections.
#[must_use]
pub fn minimal_v0_1_with_flags() -> Vector {
    let mut vector = minimal_v0_1();
    vector.name = "minimal-v0-1-with-flags";
    vector.description = "Minimal image declaring EC (required) and https (optional)";
    vector.spec.feature_flags = vec![
        FeatureFlagSpec {
            flag_id: 0x0001,
            required: true,
        },
        FeatureFlagSpec {
            flag_id: 0x0012,
            required: false,
        },
    ];
    vector
}

/// Catalog of every vector the harness runs. Add new vectors here.
#[must_use]
pub fn all_vectors() -> Vec<Vector> {
    vec![minimal_v0_1(), minimal_v0_1_with_flags()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_v0_1_has_build_history_entry() {
        let v = minimal_v0_1();
        assert_eq!(v.spec.history.len(), 1);
        assert_eq!(v.spec.history[0].op, HistoryOpSpec::Build);
    }

    #[test]
    fn minimal_v0_1_with_flags_declares_two_flags() {
        let v = minimal_v0_1_with_flags();
        assert_eq!(v.spec.feature_flags.len(), 2);
        assert!(v.spec.feature_flags[0].required);
        assert!(!v.spec.feature_flags[1].required);
    }

    #[test]
    fn all_vectors_have_at_least_one_history_entry() {
        // Spec invariant: every image has at least the build entry.
        for vector in all_vectors() {
            assert!(
                !vector.spec.history.is_empty(),
                "vector {} has no history entries",
                vector.name
            );
        }
    }

    #[test]
    fn all_vector_names_are_unique() {
        let names: Vec<_> = all_vectors().into_iter().map(|v| v.name).collect();
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(names.len(), unique.len(), "duplicate vector names");
    }

    #[test]
    fn all_vectors_have_unique_merkle_roots() {
        // Two declaratively distinct vectors must produce distinct
        // ManifestRoots (otherwise the Merkle formula is broken).
        let mut roots: std::collections::HashSet<_> = std::collections::HashSet::new();
        for vector in all_vectors() {
            let artifact = crate::builder::ManifestBuilder::new(vector.spec.clone()).build();
            assert!(
                roots.insert(artifact.merkle_root),
                "vector {} produced a Merkle root already seen",
                vector.name
            );
        }
    }
}
