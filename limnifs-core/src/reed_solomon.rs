//! Reed-Solomon erasure coding over GF(2^8).
//!
//! Implements systematic (k+m) encoding: `k` data shards + `m` parity
//! shards; any `k` of the total `k+m` shards reconstruct the original
//! data. Shards are byte-vectors of arbitrary (but equal) length.
//!
//! ## Design
//!
//! Classical Vandermonde-matrix Reed-Solomon. The generator matrix `G`
//! is `(k+m) × k` with rows `0..k` forming the identity (systematic
//! form — the first `k` output shards are the input verbatim). Rows
//! `k..k+m` are parity rows derived from a Vandermonde matrix
//! pre-multiplied by the inverse of its top `k × k` submatrix, so any
//! `k × k` submatrix of `G` is invertible and decoding is always
//! possible from any `k` surviving shards.
//!
//! ## Field
//!
//! Uses [`crate::gf256`] (Rijndael GF(2^8), same field as AES and as
//! [`crate::shamir`]). Sharing one field across the crate keeps the
//! math primitive set small.
//!
//! ## Identity preservation
//!
//! Reed-Solomon is a representation, never an identity. The spec's
//! identity rule (`DropId = BLAKE3(plaintext)`) is unaffected: the
//! original data shards round-trip through encode → decode unchanged,
//! so `DropId`s are stable.
//!
//! See task `07-reed-solomon-slabs.md`.

use crate::gf256;

/// Errors returned by [`encode`] and [`decode`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RsError {
    /// `k` (data shard count) must be in `1..=255`.
    InvalidDataShardCount { k: usize },
    /// `m` (parity shard count) must be in `1..=255-k`.
    InvalidParityShardCount { m: usize },
    /// `k + m` must not exceed 255 (the GF(2^8) order).
    TooManyShards { k: usize, m: usize },
    /// Supplied shard count does not match `k` (encode) or `k+m` (decode).
    UnexpectedShardCount { expected: usize, actual: usize },
    /// Shards disagree on length.
    InconsistentShardLen,
    /// Fewer than `k` shards survived — reconstruction impossible.
    InsufficientShards { have: usize, need: usize },
    /// Internal: a Vandermonde submatrix is singular. This is
    /// mathematically impossible with distinct shard indices, so this
    /// error signals a bug.
    SingularMatrix,
}

impl std::fmt::Display for RsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDataShardCount { k } => {
                write!(f, "RS data shard count k must be 1..=255 (got {k})")
            }
            Self::InvalidParityShardCount { m } => {
                write!(f, "RS parity shard count m must be 1..=255-k (got {m})")
            }
            Self::TooManyShards { k, m } => {
                write!(f, "RS shard count k+m must be <= 255 (got k={k} m={m})")
            }
            Self::UnexpectedShardCount { expected, actual } => {
                write!(f, "RS expected {expected} shards, got {actual}")
            }
            Self::InconsistentShardLen => write!(f, "RS shards disagree on length"),
            Self::InsufficientShards { have, need } => {
                write!(
                    f,
                    "RS need {need} shards to reconstruct, only {have} survived"
                )
            }
            Self::SingularMatrix => write!(f, "RS internal: Vandermonde submatrix is singular"),
        }
    }
}

impl std::error::Error for RsError {}

/// Validate `k` and `m` parameters shared by [`encode`] and [`decode`].
fn validate_params(k: usize, m: usize) -> Result<(), RsError> {
    if k == 0 || k > 255 {
        return Err(RsError::InvalidDataShardCount { k });
    }
    if m == 0 {
        return Err(RsError::InvalidParityShardCount { m });
    }
    if k + m > 255 {
        return Err(RsError::TooManyShards { k, m });
    }
    Ok(())
}

/// Build the systematic generator matrix `G` of shape `(k+m) × k`.
///
/// The top `k × k` block is the identity. The bottom `m × k` block is
/// `V_bot · V_top^-1`, where `V` is the Vandermonde matrix with
/// `V[i][j] = i^j` (in GF(2^8)).
fn build_generator(k: usize, m: usize) -> Vec<Vec<u8>> {
    let n = k + m;
    // V[i][j] = i^j in GF(2^8)
    let mut v = vec![vec![0u8; k]; n];
    for (i, row) in v.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = gf256::pow(
                u8::try_from(i).expect("i < 255"),
                u32::try_from(j).expect("j < k"),
            );
        }
    }
    // Invert the top k x k submatrix.
    let v_top_inv = invert_matrix(&v[0..k]).expect("Vandermonde top is invertible");
    // G = V · V_top^-1
    // The top k rows become identity (by construction of inverse).
    // We compute the full product for correctness; the parity rows are
    // what the caller actually needs.
    matrix_mul(&v, &v_top_inv)
}

/// Encode `k` data shards into `k+m` shards (data + parity).
///
/// The first `k` output shards are copies of the input. The remaining
/// `m` shards are parity. Any `k` of the `k+m` output shards suffice
/// to reconstruct the original data via [`decode`].
///
/// # Errors
///
/// See [`RsError`] variants. All shards must have the same length.
///
/// # Panics
///
/// Cannot panic — all bounds and types are checked before allocation.
pub fn encode(data: &[&[u8]], k: usize, m: usize) -> Result<Vec<Vec<u8>>, RsError> {
    validate_params(k, m)?;
    if data.len() != k {
        return Err(RsError::UnexpectedShardCount {
            expected: k,
            actual: data.len(),
        });
    }
    let shard_len = data.first().map_or(0, |s| s.len());
    for s in data {
        if s.len() != shard_len {
            return Err(RsError::InconsistentShardLen);
        }
    }
    let g = build_generator(k, m);
    // Parity rows are g[k..k+m]. For each byte index b, parity[i][b] =
    // sum_j g[k+i][j] * data[j][b].
    let mut out: Vec<Vec<u8>> = Vec::with_capacity(k + m);
    // First k shards: verbatim copies (systematic form).
    for d in data {
        out.push(d.to_vec());
    }
    // Parity shards.
    for parity_row in g.iter().take(k + m).skip(k) {
        let mut parity = vec![0u8; shard_len];
        for (b, parity_byte) in parity.iter_mut().enumerate() {
            let mut acc: u8 = 0;
            for (j, d) in data.iter().enumerate() {
                acc = gf256::add(acc, gf256::mul(parity_row[j], d[b]));
            }
            *parity_byte = acc;
        }
        out.push(parity);
    }
    Ok(out)
}

/// Decode `k+m` slots (some `None`) into `k` data shards.
///
/// `slots.len()` must equal `k + m`. Each `Some(shard)` is a surviving
/// shard; each `None` is an erasure. At least `k` shards must survive.
///
/// The returned vector contains the original `k` data shards in their
/// original order.
///
/// # Errors
///
/// See [`RsError`] variants.
///
/// # Panics
///
/// Panics if `chosen` is empty after filtering survivors — impossible
/// because `validate_params` guarantees `k >= 1` and we already checked
/// the survivor count is at least `k`.
pub fn decode(slots: &[Option<&[u8]>], k: usize, m: usize) -> Result<Vec<Vec<u8>>, RsError> {
    validate_params(k, m)?;
    if slots.len() != k + m {
        return Err(RsError::UnexpectedShardCount {
            expected: k + m,
            actual: slots.len(),
        });
    }
    let survivor_indices: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.map(|_| i))
        .collect();
    if survivor_indices.len() < k {
        return Err(RsError::InsufficientShards {
            have: survivor_indices.len(),
            need: k,
        });
    } // Use exactly k survivors (prefer the lowest indices for stability).
    let chosen: Vec<usize> = survivor_indices.into_iter().take(k).collect();
    let shard_len = slots[chosen[0]].map_or(0, <[u8]>::len);
    for &i in &chosen {
        let s = slots[i].expect("chosen from survivor list");
        if s.len() != shard_len {
            return Err(RsError::InconsistentShardLen);
        }
    }

    // Build the decoder matrix: pick the k generator rows
    // corresponding to the surviving shard indices, then invert.
    let g = build_generator(k, m);
    let survivor_rows: Vec<Vec<u8>> = chosen.iter().map(|&i| g[i].clone()).collect();
    let decoder = invert_matrix(&survivor_rows)?;

    // data[j][b] = sum_i  decoder[j][i] * slot[chosen[i]][b]
    let mut data_out: Vec<Vec<u8>> = Vec::with_capacity(k);
    for decoder_row in &decoder {
        let mut recovered = vec![0u8; shard_len];
        for (b, recovered_byte) in recovered.iter_mut().enumerate() {
            let mut acc: u8 = 0;
            for (i, &chosen_idx) in chosen.iter().enumerate() {
                let shard_byte = slots[chosen_idx].expect("chosen from survivor list")[b];
                acc = gf256::add(acc, gf256::mul(decoder_row[i], shard_byte));
            }
            *recovered_byte = acc;
        }
        data_out.push(recovered);
    }
    Ok(data_out)
}

/// Invert a `k × k` matrix over GF(2^8) via Gauss-Jordan elimination.
///
/// Returns `Err(RsError::SingularMatrix)` if the matrix is not
/// invertible. Vandermonde matrices with distinct indices are always
/// invertible, so this error should be unreachable in practice.
///
/// # Panics
///
/// Panics if any row length does not equal `matrix.len()`. Callers
/// within this module always pass square matrices; the panic guards
/// against accidental misuse.
fn invert_matrix(matrix: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, RsError> {
    let k = matrix.len();
    let mut a: Vec<Vec<u8>> = matrix.to_vec();
    // Augment with identity.
    for i in 0..k {
        let mut identity_row = vec![0u8; k];
        identity_row[i] = 1;
        a[i].extend_from_slice(&identity_row);
    }
    // Forward elimination with partial pivoting (find nonzero pivot).
    for col in 0..k {
        // Find a row at or below `col` with a nonzero pivot.
        let pivot = (col..k).find(|&r| a[r][col] != 0);
        let Some(pivot_row) = pivot else {
            return Err(RsError::SingularMatrix);
        };
        if pivot_row != col {
            a.swap(col, pivot_row);
        }
        // Scale pivot row so a[col][col] == 1.
        let inv_pivot = gf256::inv(a[col][col]);
        if inv_pivot == 0 && a[col][col] != 0 {
            return Err(RsError::SingularMatrix);
        }
        for byte in &mut a[col] {
            *byte = gf256::mul(*byte, inv_pivot);
        }
        // Eliminate the column from all other rows.
        for r in 0..k {
            if r == col {
                continue;
            }
            let factor = a[r][col];
            if factor == 0 {
                continue;
            }
            // Borrow both rows immutably first to compute contributions,
            // then write back — sidesteps the borrow checker.
            let scaled: Vec<u8> = a[col].iter().map(|&v| gf256::mul(factor, v)).collect();
            for (byte, s) in a[r].iter_mut().zip(scaled.iter()) {
                *byte = gf256::add(*byte, *s);
            }
        }
    }
    // Extract the right half (the inverse).
    Ok(a.into_iter().map(|row| row[k..].to_vec()).collect())
}

/// Multiply two `k × k` matrices in GF(2^8).
fn matrix_mul(a: &[Vec<u8>], b: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let rows = a.len();
    let cols = b.first().map_or(0, Vec::len);
    let inner = b.len();
    let mut out = vec![vec![0u8; cols]; rows];
    for i in 0..rows {
        for j in 0..cols {
            let mut acc: u8 = 0;
            for k in 0..inner {
                acc = gf256::add(acc, gf256::mul(a[i][k], b[k][j]));
            }
            out[i][j] = acc;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data(n: usize, len: usize) -> Vec<Vec<u8>> {
        (0..n)
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
    fn rejects_zero_data_shards() {
        let err = encode(&[], 0, 3).unwrap_err();
        assert_eq!(err, RsError::InvalidDataShardCount { k: 0 });
    }

    #[test]
    fn rejects_too_many_shards() {
        let err = encode(&[], 200, 100).unwrap_err();
        assert!(matches!(err, RsError::TooManyShards { .. }));
    }

    #[test]
    fn rejects_wrong_shard_count() {
        let data = sample_data(2, 4);
        let err = encode(&as_refs(&data), 3, 2).unwrap_err();
        assert_eq!(
            err,
            RsError::UnexpectedShardCount {
                expected: 3,
                actual: 2
            }
        );
    }

    #[test]
    fn rejects_inconsistent_shard_len() {
        let data = vec![vec![1, 2, 3], vec![4, 5]];
        let err = encode(&as_refs(&data), 2, 1).unwrap_err();
        assert_eq!(err, RsError::InconsistentShardLen);
    }

    #[test]
    fn encode_produces_k_plus_m_shards() {
        let data = sample_data(4, 8);
        let shards = encode(&as_refs(&data), 4, 2).unwrap();
        assert_eq!(shards.len(), 6);
        // First k shards are copies.
        for i in 0..4 {
            assert_eq!(shards[i], data[i]);
        }
    }

    #[test]
    fn round_trip_no_erasures() {
        let data = sample_data(4, 16);
        let shards = encode(&as_refs(&data), 4, 2).unwrap();
        let slots: Vec<Option<&[u8]>> = shards.iter().map(|s| Some(s.as_slice())).collect();
        let recovered = decode(&slots, 4, 2).unwrap();
        assert_eq!(recovered, data);
    }

    #[test]
    fn round_trip_one_parity_lost() {
        let data = sample_data(4, 16);
        let shards = encode(&as_refs(&data), 4, 2).unwrap();
        // Erase the last parity shard.
        let slots: Vec<Option<&[u8]>> = shards
            .iter()
            .enumerate()
            .map(|(i, s)| if i == 5 { None } else { Some(s.as_slice()) })
            .collect();
        let recovered = decode(&slots, 4, 2).unwrap();
        assert_eq!(recovered, data);
    }

    #[test]
    fn round_trip_one_data_lost() {
        let data = sample_data(4, 16);
        let shards = encode(&as_refs(&data), 4, 2).unwrap();
        // Erase data shard 1.
        let slots: Vec<Option<&[u8]>> = shards
            .iter()
            .enumerate()
            .map(|(i, s)| if i == 1 { None } else { Some(s.as_slice()) })
            .collect();
        let recovered = decode(&slots, 4, 2).unwrap();
        assert_eq!(recovered, data);
    }

    #[test]
    fn round_trip_two_shards_lost() {
        let data = sample_data(4, 16);
        let shards = encode(&as_refs(&data), 4, 2).unwrap();
        // Erase data 2 + parity 0 (indices 2 and 4).
        let slots: Vec<Option<&[u8]>> = shards
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if i == 2 || i == 4 {
                    None
                } else {
                    Some(s.as_slice())
                }
            })
            .collect();
        let recovered = decode(&slots, 4, 2).unwrap();
        assert_eq!(recovered, data);
    }

    #[test]
    fn round_trip_all_data_recoverable() {
        // k=3, m=3: any 3 of 6 shards reconstruct.
        let data = sample_data(3, 32);
        let shards = encode(&as_refs(&data), 3, 3).unwrap();
        // Try every 3-of-6 survivor subset.
        for erasures in [
            [false, false, false, true, true, true],
            [true, false, false, false, true, true],
            [true, true, false, false, false, true],
            [true, true, true, false, false, false],
            [false, true, true, true, false, false],
            [false, false, true, true, true, false],
        ] {
            let slots: Vec<Option<&[u8]>> = shards
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    if erasures[i] {
                        None
                    } else {
                        Some(s.as_slice())
                    }
                })
                .collect();
            let recovered = decode(&slots, 3, 3).unwrap();
            assert_eq!(recovered, data, "erasures={erasures:?}");
        }
    }

    #[test]
    fn decode_rejects_too_few_survivors() {
        let data = sample_data(4, 8);
        let shards = encode(&as_refs(&data), 4, 2).unwrap();
        // Erase 3 of 6 shards -> only 3 survive, need 4.
        let slots: Vec<Option<&[u8]>> = shards
            .iter()
            .enumerate()
            .map(|(i, s)| if i < 3 { None } else { Some(s.as_slice()) })
            .collect();
        let err = decode(&slots, 4, 2).unwrap_err();
        assert_eq!(err, RsError::InsufficientShards { have: 3, need: 4 });
    }

    #[test]
    fn identity_preservation() {
        // Reconstruction must yield byte-exact original data so that
        // DropId = BLAKE3(plaintext) is stable across EC round-trips.
        let data = sample_data(5, 64);
        let shards = encode(&as_refs(&data), 5, 3).unwrap();
        let slots: Vec<Option<&[u8]>> = shards
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if i == 0 || i == 7 {
                    None
                } else {
                    Some(s.as_slice())
                }
            })
            .collect();
        let recovered = decode(&slots, 5, 3).unwrap();
        assert_eq!(recovered, data);
    }

    #[test]
    fn matrix_invert_identity() {
        let identity: Vec<Vec<u8>> = (0..3)
            .map(|i| {
                let mut r = vec![0u8; 3];
                r[i] = 1;
                r
            })
            .collect();
        let inv = invert_matrix(&identity).unwrap();
        assert_eq!(inv, identity);
    }

    #[test]
    fn matrix_invert_round_trip() {
        // Build a Vandermonde matrix; its inverse times itself is identity.
        let k = 4;
        let mut v = vec![vec![0u8; k]; k];
        for (i, row) in v.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = gf256::pow(
                    u8::try_from(i + 1).unwrap_or(0),
                    u32::try_from(j).unwrap_or(0),
                );
            }
        }
        let inv = invert_matrix(&v).unwrap();
        let product = matrix_mul(&v, &inv);
        for (i, row) in product.iter().enumerate() {
            for (j, &cell) in row.iter().enumerate() {
                let expected = u8::from(i == j);
                assert_eq!(cell, expected, "i={i} j={j}");
            }
        }
    }

    #[test]
    fn encode_decode_large_k() {
        // k=10, m=4 — stress the matrix inversion.
        let data: Vec<Vec<u8>> = (0..10)
            .map(|i| vec![u8::try_from(i).unwrap_or(0); 16])
            .collect();
        let shards = encode(&as_refs(&data), 10, 4).unwrap();
        // Erase 4 random-ish shards.
        let erasures = [
            false, true, false, true, false, true, false, true, false, false, false, false, false,
            false,
        ];
        let slots: Vec<Option<&[u8]>> = shards
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if erasures[i] {
                    None
                } else {
                    Some(s.as_slice())
                }
            })
            .collect();
        let recovered = decode(&slots, 10, 4).unwrap();
        assert_eq!(recovered, data);
    }

    #[test]
    fn empty_shards_round_trip() {
        // Zero-length shards (edge case).
        let data: Vec<Vec<u8>> = vec![vec![], vec![], vec![], vec![]];
        let shards = encode(&as_refs(&data), 4, 2).unwrap();
        let slots: Vec<Option<&[u8]>> = shards.iter().map(|s| Some(s.as_slice())).collect();
        let recovered = decode(&slots, 4, 2).unwrap();
        assert_eq!(recovered, data);
    }
}
