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
            ec_params: None,
            dms_policy: None,
            history: vec![HistoryEntrySpec {
                op: HistoryOpSpec::Build,
                timestamp_ns: 0,
                inputs: Vec::new(),
                params: Vec::new(),
            }],
        },
    }
}

/// Like [`minimal_v0_1`] but declares `https:` (required) and `zstd`
/// (optional) in the feature flags. These are locator/codec flags
/// that do NOT signal optional section presence, so no EC/DMS/crypto
/// sections are emitted. Slab index still has one entry.
#[must_use]
pub fn minimal_v0_1_with_flags() -> Vector {
    let mut vector = minimal_v0_1();
    vector.name = "minimal-v0-1-with-flags";
    vector.description = "Minimal image declaring https (required) and zstd (optional)";
    vector.spec.feature_flags = vec![
        FeatureFlagSpec {
            flag_id: 0x0012,
            required: true,
        },
        FeatureFlagSpec {
            flag_id: 0x0020,
            required: false,
        },
    ];
    vector
}

/// Catalog of every vector the harness runs. Add new vectors here.
#[must_use]
pub fn all_vectors() -> Vec<Vector> {
    vec![
        minimal_v0_1(),
        minimal_v0_1_with_flags(),
        ec_params_v0_1(),
        dms_policy_v0_1(),
        inlined_metadata_v0_1(),
        multi_slab_v0_1(),
    ]
}

/// Subset of vectors that BOTH the Rust AND Python readers can parse.
/// The differential test iterates over this list; the Rust-only
/// harness iterates over [`all_vectors`].
///
/// Both readers now support all sections (including EC params and
/// DMS policy), so this returns the same set as [`all_vectors`].
/// The split is kept for future vectors that may be Rust-only
/// (e.g., crypto params until the Python reader gains that support).
#[must_use]
pub fn differential_vectors() -> Vec<Vector> {
    all_vectors()
}

/// v0.1 image with EC params section present (Reed-Solomon 4+2).
#[must_use]
pub fn ec_params_v0_1() -> Vector {
    use crate::builder::EcParamsSpec;
    let mut spec = minimal_v0_1().spec;
    // Declare EC as a required feature flag.
    spec.feature_flags = vec![FeatureFlagSpec {
        flag_id: 0x0001,
        required: true,
    }];
    spec.ec_params = Some(EcParamsSpec::new(4, 2));
    Vector {
        name: "ec-params-v0-1",
        description: "v0.1 image with Reed-Solomon EC (4, 2)",
        spec,
    }
}

/// v0.1 image with DMS policy section present (2-of-3 Shamir).
#[must_use]
pub fn dms_policy_v0_1() -> Vector {
    use crate::builder::{DmsPolicySpec, ShareRecordSpec};
    let mut spec = minimal_v0_1().spec;
    spec.feature_flags = vec![FeatureFlagSpec {
        flag_id: 0x0002,
        required: true,
    }];
    spec.dms_policy = Some(DmsPolicySpec {
        k: 2,
        n: 3,
        shares: vec![
            ShareRecordSpec {
                custodian_id: "alice".into(),
                share_data: vec![0xAA; 32],
            },
            ShareRecordSpec {
                custodian_id: "bob".into(),
                share_data: vec![0xBB; 32],
            },
            ShareRecordSpec {
                custodian_id: "carol".into(),
                share_data: vec![0xCC; 32],
            },
        ],
        reconstruction_hint: Some("Contact legal for assembly.".into()),
    });
    Vector {
        name: "dms-policy-v0-1",
        description: "v0.1 image with Shamir 2-of-3 DMS policy",
        spec,
    }
}

/// v0.1 image with inlined metadata blob instead of external locators.
#[must_use]
pub fn inlined_metadata_v0_1() -> Vector {
    let mut spec = minimal_v0_1().spec;
    spec.metadata_reference = MetadataReferenceSpec::Inlined {
        metadata: vec![0xCC; 128],
    };
    Vector {
        name: "inlined-metadata-v0-1",
        description: "v0.1 image with 128-byte inlined metadata blob",
        spec,
    }
}

/// v0.1 image with three slabs in the index, each mirrored to file and https.
#[must_use]
pub fn multi_slab_v0_1() -> Vector {
    let mut spec = minimal_v0_1().spec;
    spec.slab_index = vec![
        SlabIndexEntrySpec {
            slab_id: SlabId::new(0, [0x01; 32]),
            locators: vec![
                "file:///var/lib/limnifs/slab-0.bin".into(),
                "https://cdn/slab-0.bin".into(),
            ],
        },
        SlabIndexEntrySpec {
            slab_id: SlabId::new(1, [0x02; 32]),
            locators: vec![
                "file:///var/lib/limnifs/slab-1.bin".into(),
                "https://cdn/slab-1.bin".into(),
            ],
        },
        SlabIndexEntrySpec {
            slab_id: SlabId::new(2, [0x03; 32]),
            locators: vec![
                "file:///var/lib/limnifs/slab-2.bin".into(),
                "https://cdn/slab-2.bin".into(),
            ],
        },
    ];
    Vector {
        name: "multi-slab-v0-1",
        description: "v0.1 image with 3 mirrored slabs",
        spec,
    }
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
