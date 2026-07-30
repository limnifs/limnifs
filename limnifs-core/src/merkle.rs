//! Merkle root construction (spec §5.10, `bit-level/46-merkle-root.md`).
//!
//! The `ManifestRoot` is the canonical image identity. Readers compute
//! it from the manifest's section bytes; the manifest does not store
//! its own root. The computation is a flat BLAKE3 over a 10-byte
//! domain separator followed by 10 section hashes (330 bytes total).

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use limnifs_format::ManifestRoot;

/// 10-byte ASCII domain separator prepended to every Merkle root
/// computation. Prevents cross-protocol confusion: a BLAKE3 collision
/// with another protocol's hash output cannot be replayed as a
/// `LimniFS` `ManifestRoot`.
pub const MERKLE_DOMAIN_SEPARATOR: &[u8] = b"limnifs/v1";

/// The 10 section-hash slots that feed into [`compute_merkle_root`].
///
/// Each field is a 32-byte BLAKE3 output. For sections that are
/// absent in this manifest (e.g., `crypto_params` in a plaintext
/// image), use [`hash_empty_section`].
///
/// `metadata` is special: it is `H(metadata_blob)` (the hash of the
/// layer-2 metadata content), NOT `H(metadata_reference_section)`.
/// The metadata reference section stores this hash directly as its
/// `metadata_hash` field — readers pass that field here unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SectionHashes {
    pub metadata: [u8; 32],
    pub format_header: [u8; 32],
    pub feature_flags: [u8; 32],
    pub metadata_reference: [u8; 32],
    pub slab_index: [u8; 32],
    pub crypto_params: [u8; 32],
    pub ec_params: [u8; 32],
    pub dms_policy: [u8; 32],
    pub delta_linkage: [u8; 32],
    pub history: [u8; 32],
}

/// Compute `BLAKE3(bytes)` and return the 32-byte digest.
///
/// Use this for every section's hash slot. For absent sections, pass
/// an empty slice (`&[]`) — equivalent to calling
/// [`hash_empty_section`].
#[must_use]
pub fn hash_section(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(blake3::hash(bytes).as_bytes());
    out
}

/// Compute `BLAKE3(&[])` — the empty-section digest. Used for slots
/// whose corresponding section is absent from this manifest.
#[must_use]
pub fn hash_empty_section() -> [u8; 32] {
    hash_section(&[])
}

/// Construct a [`SectionHashes`] for an image with all optional
/// sections absent (`crypto_params`, `ec_params`, `dms_policy`,
/// `delta_linkage`).
///
/// Convenience for the common v0.1 plaintext non-delta case. The
/// caller still supplies the five required slots (`metadata`,
/// `format_header`, `feature_flags`, `metadata_reference`,
/// `slab_index`, `history` — six total since `metadata` is the
/// blob-hash).
#[must_use]
pub fn section_hashes_minimal(
    metadata: [u8; 32],
    format_header: [u8; 32],
    feature_flags: [u8; 32],
    metadata_reference: [u8; 32],
    slab_index: [u8; 32],
    history: [u8; 32],
) -> SectionHashes {
    let empty = hash_empty_section();
    SectionHashes {
        metadata,
        format_header,
        feature_flags,
        metadata_reference,
        slab_index,
        crypto_params: empty,
        ec_params: empty,
        dms_policy: empty,
        delta_linkage: empty,
        history,
    }
}

/// Compute the image's `ManifestRoot` from its 10 section hashes.
///
/// Implements the flat-construction formula from
/// [§5.10](https://github.com/limnifs/spec/blob/main/wire-format/23-manifest.md):
/// `BLAKE3("limnifs/v1" || metadata || format_header || feature_flags
/// || metadata_reference || slab_index || crypto_params || ec_params
/// || dms_policy || delta_linkage || history)`. Total input width:
/// 10 + 10 × 32 = 330 bytes. Output: 32 bytes.
///
/// Slot order is fixed; permuting two slots produces a different
/// `ManifestRoot`. The domain separator prevents cross-protocol
/// confusion.
#[must_use]
pub fn compute_merkle_root(hashes: &SectionHashes) -> ManifestRoot {
    let mut state = blake3::Hasher::new();
    state.update(MERKLE_DOMAIN_SEPARATOR);
    state.update(&hashes.metadata);
    state.update(&hashes.format_header);
    state.update(&hashes.feature_flags);
    state.update(&hashes.metadata_reference);
    state.update(&hashes.slab_index);
    state.update(&hashes.crypto_params);
    state.update(&hashes.ec_params);
    state.update(&hashes.dms_policy);
    state.update(&hashes.delta_linkage);
    state.update(&hashes.history);
    let mut out = [0u8; 32];
    out.copy_from_slice(state.finalize().as_bytes());
    ManifestRoot::from_bytes(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BLAKE3 test vector from the official BLAKE3 repo:
    /// `blake3(b"")` = `af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262`.
    /// Source: <https://github.com/BLAKE3-team/BLAKE3-specs/blob/master/test_vectors.txt>
    const BLAKE3_EMPTY_HEX: &str =
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

    fn decode_hex(hex: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("valid hex");
        }
        out
    }

    fn zero_hash() -> [u8; 32] {
        [0u8; 32]
    }

    fn one_hash(byte: u8) -> [u8; 32] {
        let mut h = [0u8; 32];
        h[0] = byte;
        h
    }

    #[test]
    fn domain_separator_is_ten_bytes_no_nul() {
        assert_eq!(MERKLE_DOMAIN_SEPARATOR.len(), 10);
        assert_eq!(MERKLE_DOMAIN_SEPARATOR, b"limnifs/v1");
        assert!(!MERKLE_DOMAIN_SEPARATOR.contains(&0));
    }

    #[test]
    fn hash_empty_section_matches_blake3_test_vector() {
        let computed = hash_empty_section();
        let expected = decode_hex(BLAKE3_EMPTY_HEX);
        assert_eq!(computed, expected, "BLAKE3(\"\") mismatch");
    }

    #[test]
    fn hash_section_long_input() {
        // blake3 of 1024 zero bytes; computed independently via the
        // blake3 crate's hash() function. The test asserts that our
        // hash_section is consistent with the crate's API.
        let input = vec![0u8; 1024];
        let computed = hash_section(&input);
        let expected_bytes = blake3::hash(&input);
        let mut expected = [0u8; 32];
        expected.copy_from_slice(expected_bytes.as_bytes());
        assert_eq!(computed, expected);
    }

    #[test]
    fn hash_section_distinct_inputs_produce_distinct_outputs() {
        let a = hash_section(b"a");
        let b = hash_section(b"b");
        let empty = hash_empty_section();
        assert_ne!(a, b);
        assert_ne!(a, empty);
        assert_ne!(b, empty);
    }

    #[test]
    fn compute_merkle_root_is_deterministic() {
        let hashes = SectionHashes {
            metadata: one_hash(1),
            format_header: one_hash(2),
            feature_flags: one_hash(3),
            metadata_reference: one_hash(4),
            slab_index: one_hash(5),
            crypto_params: one_hash(6),
            ec_params: one_hash(7),
            dms_policy: one_hash(8),
            delta_linkage: one_hash(9),
            history: one_hash(10),
        };
        let root1 = compute_merkle_root(&hashes);
        let root2 = compute_merkle_root(&hashes);
        assert_eq!(root1, root2);
    }

    #[test]
    fn compute_merkle_root_changes_when_any_slot_changes() {
        let base = SectionHashes {
            metadata: one_hash(1),
            format_header: one_hash(2),
            feature_flags: one_hash(3),
            metadata_reference: one_hash(4),
            slab_index: one_hash(5),
            crypto_params: one_hash(6),
            ec_params: one_hash(7),
            dms_policy: one_hash(8),
            delta_linkage: one_hash(9),
            history: one_hash(10),
        };
        let baseline = compute_merkle_root(&base);
        // Walk each field by index and assert mutating it changes the root.
        for slot_index in 0..10_u32 {
            let mut mutated = base;
            let zero = zero_hash();
            match slot_index {
                0 => mutated.metadata = zero,
                1 => mutated.format_header = zero,
                2 => mutated.feature_flags = zero,
                3 => mutated.metadata_reference = zero,
                4 => mutated.slab_index = zero,
                5 => mutated.crypto_params = zero,
                6 => mutated.ec_params = zero,
                7 => mutated.dms_policy = zero,
                8 => mutated.delta_linkage = zero,
                9 => mutated.history = zero,
                // No default arm: slot_index is bounded by 0..10 above.
                _ => continue,
            }
            let mutated_root = compute_merkle_root(&mutated);
            assert_ne!(
                mutated_root, baseline,
                "changing slot {slot_index} did not change the root"
            );
        }
    }

    #[test]
    fn compute_merkle_root_matches_independent_concatenation() {
        // Compute the root via our API, then compute it independently
        // by concatenating the slots manually and hashing once.
        let hashes = SectionHashes {
            metadata: one_hash(0x11),
            format_header: one_hash(0x22),
            feature_flags: one_hash(0x33),
            metadata_reference: one_hash(0x44),
            slab_index: one_hash(0x55),
            crypto_params: one_hash(0x66),
            ec_params: one_hash(0x77),
            dms_policy: one_hash(0x88),
            delta_linkage: one_hash(0x99),
            history: one_hash(0xAA),
        };
        let via_api = compute_merkle_root(&hashes);

        let mut concat = Vec::with_capacity(10 + 10 * 32);
        concat.extend_from_slice(MERKLE_DOMAIN_SEPARATOR);
        concat.extend_from_slice(&hashes.metadata);
        concat.extend_from_slice(&hashes.format_header);
        concat.extend_from_slice(&hashes.feature_flags);
        concat.extend_from_slice(&hashes.metadata_reference);
        concat.extend_from_slice(&hashes.slab_index);
        concat.extend_from_slice(&hashes.crypto_params);
        concat.extend_from_slice(&hashes.ec_params);
        concat.extend_from_slice(&hashes.dms_policy);
        concat.extend_from_slice(&hashes.delta_linkage);
        concat.extend_from_slice(&hashes.history);
        assert_eq!(concat.len(), 330);
        let mut independent = [0u8; 32];
        independent.copy_from_slice(blake3::hash(&concat).as_bytes());

        assert_eq!(via_api.as_bytes(), &independent);
    }

    #[test]
    fn minimal_image_uses_empty_section_for_absent_slots() {
        let minimal = section_hashes_minimal(
            one_hash(0x01),
            one_hash(0x02),
            one_hash(0x03),
            one_hash(0x04),
            one_hash(0x05),
            one_hash(0x06),
        );
        assert_eq!(minimal.crypto_params, hash_empty_section());
        assert_eq!(minimal.ec_params, hash_empty_section());
        assert_eq!(minimal.dms_policy, hash_empty_section());
        assert_eq!(minimal.delta_linkage, hash_empty_section());
    }

    #[test]
    fn compute_merkle_root_minimal_image_succeeds() {
        // Smoke test: a minimal image with all required sections and
        // no optional ones. The root should be a valid 32-byte value
        // that displays as b3:<base32>.
        let minimal = section_hashes_minimal(
            hash_section(b"metadata-bytes"),
            hash_section(b"header-bytes"),
            hash_section(b"flags-bytes"),
            hash_section(b"meta-ref-bytes"),
            hash_section(b"slab-index-bytes"),
            hash_section(b"history-bytes"),
        );
        let root = compute_merkle_root(&minimal);
        let text = root.to_text();
        assert!(text.starts_with("b3:"), "got: {text}");
        assert_ne!(root.as_bytes(), &[0u8; 32]);
    }

    #[test]
    fn empty_image_still_produces_nonzero_root() {
        // All-zero hashes plus empty sections still produces a
        // well-defined, non-zero root. (The domain separator and the
        // BLAKE3 of empty strings are non-zero inputs.)
        let all_empty = SectionHashes {
            metadata: hash_empty_section(),
            format_header: hash_empty_section(),
            feature_flags: hash_empty_section(),
            metadata_reference: hash_empty_section(),
            slab_index: hash_empty_section(),
            crypto_params: hash_empty_section(),
            ec_params: hash_empty_section(),
            dms_policy: hash_empty_section(),
            delta_linkage: hash_empty_section(),
            history: hash_empty_section(),
        };
        let root = compute_merkle_root(&all_empty);
        assert_ne!(root.as_bytes(), &[0u8; 32]);
    }
}
