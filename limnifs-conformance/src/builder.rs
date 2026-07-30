//! Manifest builder: declarative spec → wire bytes + computed `ManifestRoot`.
//!
//! Each `*Spec` is a declarative description of one section. The
//! [`ManifestBuilder`] encodes them in the spec's fixed section order
//! (header → feature flags → metadata reference → slab index → history
//! for v0.1 required sections; optional sections are absent and use
//! `BLAKE3("")` in their Merkle slot per the spec convention).
//!
//! The builder is the generator side of the conformance suite. It uses
//! [`limnifs_core::compute_merkle_root`] to derive each vector's
//! expected `ManifestRoot`. The harness side never links the reader —
//! it parses the bytes the builder produced and asserts identity.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use limnifs_core::{compute_merkle_root, hash_empty_section, hash_section, SectionHashes};
use limnifs_format::{ManifestRoot, SlabId};

/// Declarative description of a complete v0.1 manifest (required
/// sections only — optional sections like crypto/EC/DMS/delta are
/// absent in v0.1 plaintext non-delta images).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestSpec {
    pub header: HeaderSpec,
    pub feature_flags: Vec<FeatureFlagSpec>,
    pub metadata_reference: MetadataReferenceSpec,
    pub slab_index: Vec<SlabIndexEntrySpec>,
    pub history: Vec<HistoryEntrySpec>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeaderSpec {
    pub drop_store_version: u16,
    pub metadata_version: u16,
    pub manifest_version: u16,
}

impl HeaderSpec {
    /// The current v0.1 header versions: all layers at version 1.
    #[must_use]
    pub const fn current() -> Self {
        Self {
            drop_store_version: 1,
            metadata_version: 1,
            manifest_version: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeatureFlagSpec {
    pub flag_id: u16,
    pub required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetadataReferenceSpec {
    /// Metadata blob fetched via one or more external locators. The
    /// caller supplies the metadata blob's BLAKE3 hash directly
    /// (`metadata_hash = BLAKE3(metadata_blob)`); the builder does
    /// not recompute it (the blob's bytes live elsewhere).
    External {
        metadata_hash: [u8; 32],
        locators: Vec<String>,
    },
    /// Metadata blob inlined into the section. The builder computes
    /// `metadata_hash = BLAKE3(metadata_blob)` from the supplied bytes.
    Inlined { metadata: Vec<u8> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlabIndexEntrySpec {
    pub slab_id: SlabId,
    pub locators: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryEntrySpec {
    pub op: HistoryOpSpec,
    pub timestamp_ns: u64,
    pub inputs: Vec<ManifestRoot>,
    pub params: Vec<u8>,
}

/// Mirror of [`limnifs_core::HistoryOp`] for declarative specs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryOpSpec {
    Build,
    Delta,
    Flatten,
    Turnover,
    Deepen,
}

impl HistoryOpSpec {
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Build => 0x01,
            Self::Delta => 0x02,
            Self::Flatten => 0x03,
            Self::Turnover => 0x04,
            Self::Deepen => 0x05,
        }
    }
}

/// Builder that encodes a [`ManifestSpec`] into wire bytes and
/// derives its expected `ManifestRoot`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestBuilder {
    spec: ManifestSpec,
}

impl ManifestBuilder {
    #[must_use]
    pub const fn new(spec: ManifestSpec) -> Self {
        Self { spec }
    }

    /// Encode the spec into wire bytes plus the computed
    /// `ManifestRoot` and the section hashes that fed the computation.
    #[must_use]
    pub fn build(&self) -> ManifestArtifact {
        let mut bytes = Vec::new();
        let header_start = bytes.len();
        self.encode_header(&mut bytes);
        let header_end = bytes.len();

        let flags_start = bytes.len();
        self.encode_feature_flags(&mut bytes);
        let flags_end = bytes.len();

        let meta_ref_start = bytes.len();
        let metadata_hash = self.encode_metadata_reference(&mut bytes);
        let meta_ref_end = bytes.len();

        let slab_index_start = bytes.len();
        self.encode_slab_index(&mut bytes);
        let slab_index_end = bytes.len();

        let history_start = bytes.len();
        self.encode_history(&mut bytes);
        let history_end = bytes.len();

        let hashes = SectionHashes {
            metadata: metadata_hash,
            format_header: hash_section(&bytes[header_start..header_end]),
            feature_flags: hash_section(&bytes[flags_start..flags_end]),
            metadata_reference: hash_section(&bytes[meta_ref_start..meta_ref_end]),
            slab_index: hash_section(&bytes[slab_index_start..slab_index_end]),
            crypto_params: hash_empty_section(),
            ec_params: hash_empty_section(),
            dms_policy: hash_empty_section(),
            delta_linkage: hash_empty_section(),
            history: hash_section(&bytes[history_start..history_end]),
        };
        let merkle_root = compute_merkle_root(&hashes);
        ManifestArtifact {
            bytes,
            merkle_root,
            section_hashes: hashes,
        }
    }

    fn encode_header(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"LMFS");
        out.extend_from_slice(&self.spec.header.drop_store_version.to_le_bytes());
        out.extend_from_slice(&self.spec.header.metadata_version.to_le_bytes());
        out.extend_from_slice(&self.spec.header.manifest_version.to_le_bytes());
        // 6 reserved zero bytes per spec §5.1.
        out.extend_from_slice(&[0u8; 6]);
    }

    fn encode_feature_flags(&self, out: &mut Vec<u8>) {
        out.push(limnifs_core::FEATURE_FLAGS_SECTION_VERSION);
        let count = u32::try_from(self.spec.feature_flags.len()).expect("flag count fits u32");
        out.extend_from_slice(&count.to_le_bytes());
        for flag in &self.spec.feature_flags {
            out.extend_from_slice(&flag.flag_id.to_le_bytes());
            out.push(u8::from(flag.required));
        }
    }

    /// Encode the metadata reference section. Returns the
    /// `metadata_hash` value used (so the caller can place it in the
    /// `metadata` slot of [`SectionHashes`] without re-parsing).
    fn encode_metadata_reference(&self, out: &mut Vec<u8>) -> [u8; 32] {
        out.push(limnifs_core::METADATA_REFERENCE_SECTION_VERSION);
        let (metadata_hash, locators, inline) = match &self.spec.metadata_reference {
            MetadataReferenceSpec::External {
                metadata_hash,
                locators,
            } => (*metadata_hash, locators.clone(), None),
            MetadataReferenceSpec::Inlined { metadata } => {
                (hash_section(metadata), Vec::new(), Some(metadata.clone()))
            }
        };
        out.extend_from_slice(&metadata_hash);
        let locator_count = u32::try_from(locators.len()).expect("locator count fits u32");
        out.extend_from_slice(&locator_count.to_le_bytes());
        for locator in &locators {
            encode_locator(out, locator);
        }
        match inline {
            None => out.extend_from_slice(&0u32.to_le_bytes()),
            Some(blob) => {
                let len = u32::try_from(blob.len()).expect("inline metadata fits u32");
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(&blob);
            }
        }
        metadata_hash
    }

    fn encode_slab_index(&self, out: &mut Vec<u8>) {
        out.push(limnifs_core::SLAB_INDEX_SECTION_VERSION);
        let count = u32::try_from(self.spec.slab_index.len()).expect("slab count fits u32");
        out.extend_from_slice(&count.to_le_bytes());
        for entry in &self.spec.slab_index {
            let slab_bytes = entry.slab_id.to_bytes();
            out.extend_from_slice(&slab_bytes);
            let locator_count =
                u32::try_from(entry.locators.len()).expect("locator count fits u32");
            out.extend_from_slice(&locator_count.to_le_bytes());
            for locator in &entry.locators {
                encode_locator(out, locator);
            }
        }
    }

    fn encode_history(&self, out: &mut Vec<u8>) {
        out.push(limnifs_core::HISTORY_SECTION_VERSION);
        let count = u32::try_from(self.spec.history.len()).expect("history count fits u32");
        out.extend_from_slice(&count.to_le_bytes());
        for entry in &self.spec.history {
            out.push(entry.op.to_byte());
            out.extend_from_slice(&entry.timestamp_ns.to_le_bytes());
            let input_count = u32::try_from(entry.inputs.len()).expect("input count fits u32");
            out.extend_from_slice(&input_count.to_le_bytes());
            for input in &entry.inputs {
                out.extend_from_slice(input.as_bytes());
            }
            let params_len = u32::try_from(entry.params.len()).expect("params len fits u32");
            out.extend_from_slice(&params_len.to_le_bytes());
            out.extend_from_slice(&entry.params);
        }
    }
}

fn encode_locator(out: &mut Vec<u8>, uri: &str) {
    let bytes = uri.as_bytes();
    let len = u32::try_from(bytes.len()).expect("locator URI fits u32");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
}

/// Output of [`ManifestBuilder::build`]: the wire bytes plus the
/// expected `ManifestRoot` and the section hashes that produced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestArtifact {
    pub bytes: Vec<u8>,
    pub merkle_root: ManifestRoot,
    pub section_hashes: SectionHashes,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_spec_current_is_v1() {
        let h = HeaderSpec::current();
        assert_eq!(h.drop_store_version, 1);
        assert_eq!(h.metadata_version, 1);
        assert_eq!(h.manifest_version, 1);
    }

    #[test]
    fn history_op_spec_byte_values_match_spec() {
        assert_eq!(HistoryOpSpec::Build.to_byte(), 0x01);
        assert_eq!(HistoryOpSpec::Delta.to_byte(), 0x02);
        assert_eq!(HistoryOpSpec::Flatten.to_byte(), 0x03);
        assert_eq!(HistoryOpSpec::Turnover.to_byte(), 0x04);
        assert_eq!(HistoryOpSpec::Deepen.to_byte(), 0x05);
    }

    #[test]
    fn build_empty_inputs_produces_valid_bytes() {
        // Construct a minimal spec with zero entries everywhere they
        // are allowed to be zero (flags, slab_index) — except history
        // which must have >= 1 entry per spec.
        let spec = ManifestSpec {
            header: HeaderSpec::current(),
            feature_flags: Vec::new(),
            metadata_reference: MetadataReferenceSpec::External {
                metadata_hash: [0xAA; 32],
                locators: vec!["file:///m.bin".into()],
            },
            slab_index: Vec::new(),
            history: vec![HistoryEntrySpec {
                op: HistoryOpSpec::Build,
                timestamp_ns: 0,
                inputs: Vec::new(),
                params: Vec::new(),
            }],
        };
        let artifact = ManifestBuilder::new(spec).build();
        // Header (16) + flags (5) + meta_ref (1 + 32 + 4 + 4 + 23 + 4 = 68) + slab_index (5) + history (1 + 4 + 1 + 8 + 4 + 4 = 22) = 116
        // locator URI "file:///m.bin" is 13 bytes; prefix is 4. So 13 + 4 = 17.
        // meta_ref total = 1 + 32 + 4 + 4 + 13 + 4 = 58
        // Total: 16 + 5 + 58 + 5 + 22 = 106
        assert_eq!(artifact.bytes.len(), 106);
        assert_ne!(artifact.merkle_root.as_bytes(), &[0u8; 32]);
    }

    #[test]
    fn build_inlined_metadata_hashes_the_blob() {
        let blob = vec![0xBB; 64];
        let expected_hash = hash_section(&blob);
        let spec = ManifestSpec {
            header: HeaderSpec::current(),
            feature_flags: Vec::new(),
            metadata_reference: MetadataReferenceSpec::Inlined { metadata: blob },
            slab_index: Vec::new(),
            history: vec![HistoryEntrySpec {
                op: HistoryOpSpec::Build,
                timestamp_ns: 0,
                inputs: Vec::new(),
                params: Vec::new(),
            }],
        };
        let artifact = ManifestBuilder::new(spec).build();
        assert_eq!(artifact.section_hashes.metadata, expected_hash);
    }

    #[test]
    fn build_is_deterministic_for_same_spec() {
        let spec = ManifestSpec {
            header: HeaderSpec::current(),
            feature_flags: vec![FeatureFlagSpec {
                flag_id: 0x0001,
                required: true,
            }],
            metadata_reference: MetadataReferenceSpec::External {
                metadata_hash: [0x11; 32],
                locators: vec!["file:///m.bin".into()],
            },
            slab_index: vec![SlabIndexEntrySpec {
                slab_id: SlabId::new(7, [0x22; 32]),
                locators: vec!["file:///s.bin".into()],
            }],
            history: vec![HistoryEntrySpec {
                op: HistoryOpSpec::Build,
                timestamp_ns: 0,
                inputs: Vec::new(),
                params: Vec::new(),
            }],
        };
        let a1 = ManifestBuilder::new(spec.clone()).build();
        let a2 = ManifestBuilder::new(spec).build();
        assert_eq!(a1.bytes, a2.bytes);
        assert_eq!(a1.merkle_root, a2.merkle_root);
    }
}
