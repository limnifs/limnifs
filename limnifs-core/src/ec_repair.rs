//! Slab erasure-coding repair — reconstruct missing shards from
//! surviving ones and re-emit them to the caller.
//!
//! ## Design
//!
//! Thin orchestration layer over [`crate::reed_solomon`]. The heavy
//! lifting (matrix inversion, Lagrange interpolation in GF(2^8))
//! lives in `reed_solomon.rs`. This module wires Reed-Solomon to the
//! slab domain: a slab is a `(k+m)`-shard erasure-coded blob; some
//! shards are missing (erased); repair reconstructs them.
//!
//! ## Algorithm
//!
//! 1. Caller supplies the surviving shards (with their indices) and
//!    the slab's `(k, m)` parameters.
//! 2. [`repair_shards`] runs Reed-Solomon decode to recover the `k`
//!    original data shards.
//! 3. Re-encodes parity via [`crate::reed_solomon::encode`].
//! 4. Returns the full `(k+m)`-shard set: originals + reconstructed.
//!
//! ## Repair is offline
//!
//! Repair is a background/CLI operation, never on the hot read path.
//! Reads fail over to surviving shards transparently; repair only
//! runs when the operator decides redundancy is too low.
//!
//! ## Identity preservation
//!
//! Repair reconstructs byte-exact original shards, so `DropId`s stay
//! stable across repair cycles. The acceptance criterion "image
//! remains fully readable with m shards absent" follows directly.
//!
//! See task `07-ec-repair.md`.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use crate::reed_solomon::{self, RsError};

/// A slab with some shards possibly missing.
#[derive(Clone, Debug)]
pub struct DegradedSlab<'a> {
    /// Slab data shards (length k). `None` = erasure.
    pub data_shards: Vec<Option<&'a [u8]>>,
    /// Slab parity shards (length m). `None` = erasure.
    pub parity_shards: Vec<Option<&'a [u8]>>,
    /// Original k parameter.
    pub k: usize,
    /// Original m parameter.
    pub m: usize,
}

/// Result of a repair operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepairResult {
    /// All reconstructed data shards (length k, in original order).
    pub data_shards: Vec<Vec<u8>>,
    /// All reconstructed parity shards (length m, in original order).
    pub parity_shards: Vec<Vec<u8>>,
    /// Indices of shards that were missing and got reconstructed
    /// (mix of data and parity).
    pub reconstructed: Vec<usize>,
}

/// Errors from [`repair_shards`].
#[derive(Debug)]
pub enum RepairError {
    /// Wraps a Reed-Solomon error.
    Rs(RsError),
    /// `data_shards.len() != k` or `parity_shards.len() != m`.
    WrongArity { expected: usize, actual: usize },
    /// Surviving shards disagree on length.
    InconsistentShardLen,
}

impl std::fmt::Display for RepairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rs(e) => write!(f, "repair: Reed-Solomon failure: {e}"),
            Self::WrongArity { expected, actual } => {
                write!(f, "repair: expected {expected} shards, got {actual}")
            }
            Self::InconsistentShardLen => write!(f, "repair: surviving shards disagree on length"),
        }
    }
}

impl std::error::Error for RepairError {}

impl From<RsError> for RepairError {
    fn from(e: RsError) -> Self {
        Self::Rs(e)
    }
}

/// Repair a degraded slab: reconstruct missing shards.
///
/// Accepts the surviving data and parity shards. Returns the full
/// `(k+m)`-shard set plus the list of reconstructed indices.
///
/// # Errors
///
/// See [`RepairError`] variants.
pub fn repair_shards(degraded: &DegradedSlab<'_>) -> Result<RepairResult, RepairError> {
    if degraded.data_shards.len() != degraded.k {
        return Err(RepairError::WrongArity {
            expected: degraded.k,
            actual: degraded.data_shards.len(),
        });
    }
    if degraded.parity_shards.len() != degraded.m {
        return Err(RepairError::WrongArity {
            expected: degraded.m,
            actual: degraded.parity_shards.len(),
        });
    }

    // Build the (k+m)-slot vector for reed_solomon::decode.
    let mut slots: Vec<Option<&[u8]>> = Vec::with_capacity(degraded.k + degraded.m);
    for s in &degraded.data_shards {
        slots.push(*s);
    }
    for s in &degraded.parity_shards {
        slots.push(*s);
    }

    // Record which slots were missing.
    let missing: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| if s.is_none() { Some(i) } else { None })
        .collect();

    // Decode recovers the original k data shards.
    let data = reed_solomon::decode(&slots, degraded.k, degraded.m)?;

    // Re-encode to recover parity (in case any parity shards were missing).
    let data_refs: Vec<&[u8]> = data.iter().map(Vec::as_slice).collect();
    let full = reed_solomon::encode(&data_refs, degraded.k, degraded.m)?;

    let mut parity_shards = Vec::with_capacity(degraded.m);
    for p in full.iter().skip(degraded.k) {
        parity_shards.push(p.clone());
    }

    Ok(RepairResult {
        data_shards: data,
        parity_shards,
        reconstructed: missing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_shards(k: usize, len: usize) -> Vec<Vec<u8>> {
        (0..k)
            .map(|i| {
                let base = u8::try_from(i).unwrap_or(0);
                (0..len)
                    .map(|j| base.wrapping_add(u8::try_from(j).unwrap_or(0)))
                    .collect()
            })
            .collect()
    }

    fn as_refs(data: &[Vec<u8>]) -> Vec<&[u8]> {
        data.iter().map(Vec::as_slice).collect()
    }

    #[test]
    fn repair_one_missing_data_shard() {
        let k = 4;
        let m = 2;
        let original = sample_shards(k, 32);
        let encoded = reed_solomon::encode(&as_refs(&original), k, m).unwrap();

        // Lose data shard 2.
        let data_shards: Vec<Option<&[u8]>> = (0..k)
            .map(|i| {
                if i == 2 {
                    None
                } else {
                    Some(encoded[i].as_slice())
                }
            })
            .collect();
        let parity_shards: Vec<Option<&[u8]>> =
            (k..k + m).map(|i| Some(encoded[i].as_slice())).collect();

        let degraded = DegradedSlab {
            data_shards,
            parity_shards,
            k,
            m,
        };
        let repair = repair_shards(&degraded).unwrap();
        assert_eq!(repair.data_shards, original);
        // Parity matches the freshly encoded set.
        for i in 0..m {
            assert_eq!(repair.parity_shards[i], encoded[k + i]);
        }
        assert_eq!(repair.reconstructed, vec![2]);
    }

    #[test]
    fn repair_two_missing_parity_shards() {
        let k = 3;
        let m = 3;
        let original = sample_shards(k, 16);
        let encoded = reed_solomon::encode(&as_refs(&original), k, m).unwrap();

        // All data present, lose parities 0 and 2 (indices k and k+2).
        let data_shards: Vec<Option<&[u8]>> = (0..k).map(|i| Some(encoded[i].as_slice())).collect();
        let parity_shards: Vec<Option<&[u8]>> = (0..m)
            .map(|i| {
                if i == 0 || i == 2 {
                    None
                } else {
                    Some(encoded[k + i].as_slice())
                }
            })
            .collect();

        let degraded = DegradedSlab {
            data_shards,
            parity_shards,
            k,
            m,
        };
        let repair = repair_shards(&degraded).unwrap();
        assert_eq!(repair.data_shards, original);
        for i in 0..m {
            assert_eq!(repair.parity_shards[i], encoded[k + i]);
        }
        // Parities k+0 and k+2 were reconstructed.
        assert_eq!(repair.reconstructed, vec![k, k + 2]);
    }

    #[test]
    fn repair_m_missing_shards_image_still_readable() {
        // The acceptance criterion: image remains fully readable
        // with m shards absent.
        let k = 4;
        let m = 2;
        let original = sample_shards(k, 64);
        let encoded = reed_solomon::encode(&as_refs(&original), k, m).unwrap();

        // Lose 2 shards (1 data + 1 parity). Reader has 4 surviving shards.
        let data_shards: Vec<Option<&[u8]>> = (0..k)
            .map(|i| {
                if i == 1 {
                    None
                } else {
                    Some(encoded[i].as_slice())
                }
            })
            .collect();
        let parity_shards: Vec<Option<&[u8]>> = (k..k + m)
            .map(|i| {
                if i == k + 1 {
                    None
                } else {
                    Some(encoded[i].as_slice())
                }
            })
            .collect();

        let degraded = DegradedSlab {
            data_shards,
            parity_shards,
            k,
            m,
        };
        let repair = repair_shards(&degraded).unwrap();
        // Reconstructed data shards equal the originals byte-for-byte.
        assert_eq!(repair.data_shards, original);
        assert_eq!(repair.reconstructed.len(), m);
    }

    #[test]
    fn repair_too_many_erasures_fails() {
        let k = 4;
        let m = 2;
        let original = sample_shards(k, 32);
        let encoded = reed_solomon::encode(&as_refs(&original), k, m).unwrap();

        // Lose 3 shards (only 3 survive). Cannot repair.
        let data_shards: Vec<Option<&[u8]>> = (0..k)
            .map(|i| {
                if i < 3 {
                    None
                } else {
                    Some(encoded[i].as_slice())
                }
            })
            .collect();
        let parity_shards: Vec<Option<&[u8]>> =
            (k..k + m).map(|i| Some(encoded[i].as_slice())).collect();

        let degraded = DegradedSlab {
            data_shards,
            parity_shards,
            k,
            m,
        };
        let err = repair_shards(&degraded).unwrap_err();
        assert!(matches!(
            err,
            RepairError::Rs(RsError::InsufficientShards { .. })
        ));
    }

    #[test]
    fn repair_no_erasures_is_identity() {
        let k = 3;
        let m = 2;
        let original = sample_shards(k, 16);
        let encoded = reed_solomon::encode(&as_refs(&original), k, m).unwrap();

        let data_shards: Vec<Option<&[u8]>> = (0..k).map(|i| Some(encoded[i].as_slice())).collect();
        let parity_shards: Vec<Option<&[u8]>> =
            (k..k + m).map(|i| Some(encoded[i].as_slice())).collect();

        let degraded = DegradedSlab {
            data_shards,
            parity_shards,
            k,
            m,
        };
        let repair = repair_shards(&degraded).unwrap();
        assert_eq!(repair.data_shards, original);
        for i in 0..m {
            assert_eq!(repair.parity_shards[i], encoded[k + i]);
        }
        assert!(
            repair.reconstructed.is_empty(),
            "no reconstructions when nothing was missing"
        );
    }

    #[test]
    fn repair_wrong_arity_rejected() {
        let k = 4;
        let m = 2;
        let original = sample_shards(k, 16);
        let encoded = reed_solomon::encode(&as_refs(&original), k, m).unwrap();
        // Pass 5 data shards instead of 4.
        let data_shards: Vec<Option<&[u8]>> = (0..=k)
            .map(|i| Some(encoded[i.min(k + m - 1)].as_slice()))
            .collect();
        let parity_shards: Vec<Option<&[u8]>> =
            (k..k + m).map(|i| Some(encoded[i].as_slice())).collect();

        let degraded = DegradedSlab {
            data_shards,
            parity_shards,
            k,
            m,
        };
        let err = repair_shards(&degraded).unwrap_err();
        assert!(matches!(err, RepairError::WrongArity { .. }));
    }

    #[test]
    fn repair_preserves_drop_ids() {
        // Identity rule: reconstructed shards have the same DropIds
        // as the originals.
        let k = 5;
        let m = 3;
        let original = sample_shards(k, 32);
        let encoded = reed_solomon::encode(&as_refs(&original), k, m).unwrap();

        // Lose data shards 0 and 2, parity 1.
        let data_shards: Vec<Option<&[u8]>> = (0..k)
            .map(|i| {
                if i == 0 || i == 2 {
                    None
                } else {
                    Some(encoded[i].as_slice())
                }
            })
            .collect();
        let parity_shards: Vec<Option<&[u8]>> = (k..k + m)
            .map(|i| {
                if i == k + 1 {
                    None
                } else {
                    Some(encoded[i].as_slice())
                }
            })
            .collect();

        let degraded = DegradedSlab {
            data_shards,
            parity_shards,
            k,
            m,
        };
        let repair = repair_shards(&degraded).unwrap();
        // Every reconstructed data shard's BLAKE3 matches the original.
        for (i, shard) in repair.data_shards.iter().enumerate() {
            let orig_hash = blake3::hash(&original[i]);
            let repaired_hash = blake3::hash(shard);
            assert_eq!(orig_hash, repaired_hash, "data shard {i} DropId mismatch");
        }
    }
}
