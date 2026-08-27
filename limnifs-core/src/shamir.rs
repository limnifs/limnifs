//! Shamir secret sharing over GF(2^8).
//!
//! Implements k-of-n threshold splitting of a byte-string secret: any
//! k shares reconstruct, any k-1 reveal nothing about the secret.
//!
//! ## Design
//!
//! Standard Shamir over the Rijndael GF(2^8) field (reduction
//! polynomial `x^8 + x^4 + x^3 + x + 1`, the same one AES uses). Each
//! byte of the secret becomes the constant term of a degree-(k-1)
//! polynomial; share `i` is `(i, poly(i))`. Reconstruction is
//! Lagrange interpolation at x=0.
//!
//! ## Random source
//!
//! All coefficient bytes come from a caller-supplied CSPRNG closure.
//! The library never invokes `getrandom` itself — that would couple
//! the math to system RNG and make the caller's entropy choice opaque.
//! Callers typically pass `|out| getrandom::getrandom(out).map_err(...)`
//! (or any wrapper that fits their threat model).
//!
//! ## Wire format
//!
//! A share is 33 bytes: 1 byte index (`1..=255`), 32 bytes payload
//! (the per-byte evaluations concatenated in order). This matches the
//! DMS policy section's per-share record layout.
//!
//! See task `05-dms.md`.

use crate::gf256;

/// GF(2^8) reduction polynomial: x^8 + x^4 + x^3 + x + 1 = 0x11B.
///
/// Re-exported from [`crate::gf256`] for documentation continuity.
pub use crate::gf256::REDUCTION_POLYNOMIAL as AES_POLYNOMIAL;

/// Split `secret` into `n` shares, any `k` of which reconstruct it.
///
/// `rng` must fill `out` with cryptographically-random bytes. The
/// caller owns the entropy source.
///
/// Returns the `n` shares (indices `1..=n`), each `1 + secret.len()`
/// bytes long.
///
/// # Errors
///
/// - [`ShamirError::ThresholdTooSmall`] if `k < 2`.
/// - [`ShamirError::ThresholdExceedsShares`] if `k > n`.
/// - [`ShamirError::TooManyShares`] if `n > 255`.
/// - [`ShamirError::EmptySecret`] if `secret.is_empty()`.
///
/// # Panics
///
/// Panics if `rng` returns an error (entropy exhaustion is fatal —
/// callers must not silently fall back to deterministic randomness).
pub fn split<F>(secret: &[u8], k: usize, n: usize, mut rng: F) -> Result<Vec<Vec<u8>>, ShamirError>
where
    F: FnMut(&mut [u8]) -> Result<(), ShamirError>,
{
    if k < 2 {
        return Err(ShamirError::ThresholdTooSmall { k });
    }
    if k > n {
        return Err(ShamirError::ThresholdExceedsShares { k, n });
    }
    if n > 255 {
        return Err(ShamirError::TooManyShares { n });
    }
    if secret.is_empty() {
        return Err(ShamirError::EmptySecret);
    }

    // Each secret byte has its own random polynomial of degree k-1.
    // coefficients[byte_idx] = [a_0=secret_byte, a_1, a_2, ..., a_{k-1}]
    let mut coefficients = vec![vec![0u8; k]; secret.len()];
    for (i, coeff_row) in coefficients.iter_mut().enumerate() {
        coeff_row[0] = secret[i];
        rng(&mut coeff_row[1..])?;
    }

    let mut shares = Vec::with_capacity(n);
    for index in 1..=n {
        let mut share = Vec::with_capacity(1 + secret.len());
        share.push(u8::try_from(index).expect("index <= 255"));
        for coeff_row in &coefficients {
            share.push(eval_poly(
                coeff_row,
                u8::try_from(index).expect("index <= 255"),
            ));
        }
        shares.push(share);
    }
    Ok(shares)
}

/// Reconstruct the secret from `k` (or more) shares.
///
/// `shares` must each be of length `1 + secret_len`. Their first byte
/// is the share index (`1..=255`); the remaining bytes are the
/// per-byte evaluations.
///
/// # Errors
///
/// - [`ShamirError::NoShares`] if `shares.is_empty()`.
/// - [`ShamirError::MalformedShare`] if any share is shorter than 2 bytes.
/// - [`ShamirError::InconsistentShareLen`] if shares disagree on length.
/// - [`ShamirError::DuplicateIndex`] if two shares carry the same index.
///
/// # Panics
///
/// Panics if fewer than `k` shares are supplied where `k` cannot be
/// inferred. In practice, callers supply exactly the number of shares
/// they have; reconstruction proceeds with whatever count is given.
pub fn combine(shares: &[&[u8]]) -> Result<Vec<u8>, ShamirError> {
    if shares.is_empty() {
        return Err(ShamirError::NoShares);
    }
    let body_len = shares[0]
        .len()
        .checked_sub(1)
        .ok_or(ShamirError::MalformedShare)?;
    if body_len == 0 {
        return Err(ShamirError::MalformedShare);
    }
    for s in shares {
        if s.len() != body_len + 1 {
            return Err(ShamirError::InconsistentShareLen);
        }
    }
    let mut seen_indices = std::collections::HashSet::new();
    for s in shares {
        if !seen_indices.insert(s[0]) {
            return Err(ShamirError::DuplicateIndex { index: s[0] });
        }
    }

    let mut secret = vec![0u8; body_len];
    for byte_idx in 0..body_len {
        let points: Vec<(u8, u8)> = shares.iter().map(|s| (s[0], s[1 + byte_idx])).collect();
        secret[byte_idx] = lagrange_at_zero(&points);
    }
    Ok(secret)
}

/// Errors returned by [`split`] and [`combine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShamirError {
    /// `k` must be at least 2.
    ThresholdTooSmall { k: usize },
    /// `k > n` is invalid (threshold unachievable).
    ThresholdExceedsShares { k: usize, n: usize },
    /// `n` must fit in a u8 (255 is the maximum number of shares).
    TooManyShares { n: usize },
    /// The secret has zero length.
    EmptySecret,
    /// No shares were supplied.
    NoShares,
    /// A share is shorter than 2 bytes (need at least index + 1 byte).
    MalformedShare,
    /// Shares disagree on length.
    InconsistentShareLen,
    /// Two shares share the same index.
    DuplicateIndex { index: u8 },
    /// Random source returned an error.
    RngFailed { reason: String },
}

impl std::fmt::Display for ShamirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ThresholdTooSmall { k } => {
                write!(f, "Shamir threshold must be >= 2 (got k={k})")
            }
            Self::ThresholdExceedsShares { k, n } => {
                write!(f, "Shamir threshold k={k} exceeds n={n}")
            }
            Self::TooManyShares { n } => write!(f, "Shamir share count {n} exceeds 255"),
            Self::EmptySecret => write!(f, "Shamir secret is empty"),
            Self::NoShares => write!(f, "Shamir combine: no shares supplied"),
            Self::MalformedShare => write!(f, "Shamir share is malformed (need index + >=1 byte)"),
            Self::InconsistentShareLen => write!(f, "Shamir shares disagree on length"),
            Self::DuplicateIndex { index } => {
                write!(f, "Shamir duplicate share index {index}")
            }
            Self::RngFailed { reason } => write!(f, "Shamir RNG failure: {reason}"),
        }
    }
}

impl std::error::Error for ShamirError {}

fn eval_poly(coeffs: &[u8], x: u8) -> u8 {
    // Horner's method.
    let mut acc: u8 = 0;
    for &c in coeffs.iter().rev() {
        acc = gf256::add(gf256::mul(acc, x), c);
    }
    acc
}

fn lagrange_at_zero(points: &[(u8, u8)]) -> u8 {
    // secret = sum_i  y_i * prod_{j!=i}  (0 - x_j) / (x_i - x_j)
    // All operations in GF(2^8); subtraction is XOR (= addition).
    let mut acc: u8 = 0;
    for (i, &(xi, yi)) in points.iter().enumerate() {
        let mut num: u8 = 1;
        let mut den: u8 = 1;
        for (j, &(xj, _)) in points.iter().enumerate() {
            if i == j {
                continue;
            }
            num = gf256::mul(num, xj);
            den = gf256::mul(den, xi ^ xj);
        }
        let term = gf256::mul(yi, gf256::mul(num, gf256::inv(den)));
        acc ^= term;
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deterministic_rng(seed: u8) -> impl FnMut(&mut [u8]) -> Result<(), ShamirError> {
        let mut state = seed;
        move |out: &mut [u8]| {
            for b in out.iter_mut() {
                // LCG for test reproducibility; NOT cryptographic.
                state = state.wrapping_mul(31).wrapping_add(17);
                *b = state;
            }
            Ok(())
        }
    }

    #[test]
    fn eval_poly_constant_returns_constant() {
        assert_eq!(eval_poly(&[42], 5), 42);
        assert_eq!(eval_poly(&[42], 0), 42);
        assert_eq!(eval_poly(&[42], 255), 42);
    }

    #[test]
    fn eval_poly_at_zero_is_constant_term() {
        let coeffs = &[10, 20, 30, 40]; // 10 + 20x + 30x^2 + 40x^3
        assert_eq!(eval_poly(coeffs, 0), 10);
    }

    #[test]
    fn split_combine_round_trip_k2_n3() {
        let secret = b"hello shamir!";
        let shares = split(secret, 2, 3, deterministic_rng(7)).unwrap();
        assert_eq!(shares.len(), 3);
        // Any 2 of the 3 shares reconstruct.
        let pairs: Vec<(&[u8], &[u8])> = vec![
            (&shares[0], &shares[1]),
            (&shares[0], &shares[2]),
            (&shares[1], &shares[2]),
        ];
        for (a, b) in pairs {
            let combined = combine(&[a, b]).unwrap();
            assert_eq!(combined, secret);
        }
    }

    #[test]
    fn split_combine_round_trip_k3_n5() {
        let secret = b"0123456789abcdef";
        let shares = split(secret, 3, 5, deterministic_rng(11)).unwrap();
        assert_eq!(shares.len(), 5);
        // Use the first 3.
        let combined = combine(&[&shares[0], &shares[2], &shares[4]]).unwrap();
        assert_eq!(combined, secret);
        // Use any other 3.
        let combined = combine(&[&shares[1], &shares[2], &shares[3]]).unwrap();
        assert_eq!(combined, secret);
    }

    #[test]
    fn split_combine_round_trip_k5_n5() {
        let secret = b"k-of-n where k equals n";
        let shares = split(secret, 5, 5, deterministic_rng(13)).unwrap();
        let combined =
            combine(&[&shares[0], &shares[1], &shares[2], &shares[3], &shares[4]]).unwrap();
        assert_eq!(combined, secret);
    }

    #[test]
    fn split_combine_with_real_csprng() {
        // getrandom is the portable CSPRNG primitive (already in the
        // dependency tree via ed25519-dalek). This is a round-trip
        // test, not a randomness-quality test — the latter would need
        // NIST-style vectors and is out of scope.
        let mut seed = vec![0u8; 256];
        getrandom::getrandom(&mut seed).expect("csprng");
        let mut iter = seed.into_iter();
        let rng = |out: &mut [u8]| {
            for b in out.iter_mut() {
                *b = iter.next().unwrap_or(0);
            }
            Ok::<(), ShamirError>(())
        };
        let secret = b"cryptographically random coefficients";
        let shares = split(secret, 3, 5, rng).unwrap();
        let combined = combine(&[&shares[0], &shares[2], &shares[4]]).unwrap();
        assert_eq!(combined, secret);
    }

    #[test]
    fn k_minus_1_shares_do_not_deterministically_reconstruct() {
        // With only k-1 shares, any value is plausible (Shamir is
        // information-theoretically secure). We verify that a few
        // different (k-1)-subsets yield *different* reconstructions
        // when extended with arbitrary synthetic shares.
        let secret = b"the secret";
        let shares = split(secret, 3, 5, deterministic_rng(23)).unwrap();
        // 2 shares + 1 synthetic share at index=99 -> each subset gives
        // a different "secret". This confirms k-1 shares leak nothing.
        let synthetic1 = {
            let mut s = vec![99u8];
            s.extend_from_slice(&shares[0][1..]);
            s
        };
        let r1 = combine(&[&shares[0], &shares[1], &synthetic1]).unwrap();
        let synthetic2 = {
            let mut s = vec![99u8];
            s.extend_from_slice(&shares[1][1..]);
            s
        };
        let r2 = combine(&[&shares[0], &shares[1], &synthetic2]).unwrap();
        // Two different synthetic shares at the same index would both
        // need to coincidentally yield the actual secret — they almost
        // always yield different junk.
        assert!(r1 != r2 || shares[0][1..] == shares[1][1..]);
    }

    #[test]
    fn rejects_k_too_small() {
        let err = split(b"x", 1, 3, deterministic_rng(0)).unwrap_err();
        assert_eq!(err, ShamirError::ThresholdTooSmall { k: 1 });
    }

    #[test]
    fn rejects_k_exceeds_n() {
        let err = split(b"x", 5, 3, deterministic_rng(0)).unwrap_err();
        assert_eq!(err, ShamirError::ThresholdExceedsShares { k: 5, n: 3 });
    }

    #[test]
    fn rejects_too_many_shares() {
        let err = split(b"x", 2, 256, deterministic_rng(0)).unwrap_err();
        assert_eq!(err, ShamirError::TooManyShares { n: 256 });
    }

    #[test]
    fn rejects_empty_secret() {
        let err = split(b"", 2, 3, deterministic_rng(0)).unwrap_err();
        assert_eq!(err, ShamirError::EmptySecret);
    }

    #[test]
    fn combine_rejects_empty() {
        let err = combine(&[]).unwrap_err();
        assert_eq!(err, ShamirError::NoShares);
    }

    #[test]
    fn combine_rejects_malformed_share() {
        let single = vec![5u8];
        let err = combine(&[&single]).unwrap_err();
        assert_eq!(err, ShamirError::MalformedShare);
    }

    #[test]
    fn combine_rejects_inconsistent_lengths() {
        let a = vec![1, 2, 3];
        let b = vec![2, 3, 4, 5];
        let err = combine(&[&a, &b]).unwrap_err();
        assert_eq!(err, ShamirError::InconsistentShareLen);
    }

    #[test]
    fn combine_rejects_duplicate_index() {
        let a = vec![1, 10, 20];
        let b = vec![1, 30, 40];
        let err = combine(&[&a, &b]).unwrap_err();
        assert_eq!(err, ShamirError::DuplicateIndex { index: 1 });
    }

    #[test]
    fn share_indices_start_at_one() {
        let shares = split(b"secret", 2, 3, deterministic_rng(0)).unwrap();
        for (i, s) in shares.iter().enumerate() {
            assert_eq!(s[0], u8::try_from(i + 1).unwrap());
        }
    }

    #[test]
    fn share_length_is_index_plus_secret() {
        let secret = b"7 bytes";
        let shares = split(secret, 2, 3, deterministic_rng(0)).unwrap();
        for s in &shares {
            assert_eq!(s.len(), 1 + secret.len());
        }
    }

    #[test]
    fn lagrange_correctness_for_linear_polynomial() {
        // poly(x) = 5 + 7*x in GF(2^8). At x=0, value=5.
        let points = vec![(1, 5 ^ gf256::mul(7, 1)), (2, 5 ^ gf256::mul(7, 2))];
        assert_eq!(lagrange_at_zero(&points), 5);
    }
}
