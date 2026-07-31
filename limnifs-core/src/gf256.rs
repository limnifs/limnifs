//! GF(2^8) arithmetic — shared field for Shamir secret sharing and
//! Reed-Solomon erasure coding.
//!
//! Uses the Rijndael reduction polynomial `x^8 + x^4 + x^3 + x + 1`
//! (0x11B), the same field AES uses. This choice keeps the math
//! primitive set small (no second field for downstream code), at the
//! cost of needing this polynomial wherever `gf256` is used.
//!
//! ## Why a single field?
//!
//! Both Shamir and Reed-Solomon need a finite field large enough to
//! index 256 shards / shares. Sharing one field means:
//!
//! - One log/exp table (or one shift-XOR loop) implementation.
//! - Consistent inverse semantics across crates.
//! - Smaller dep tree at every call site.
//!
//! The AES polynomial is well-studied, has no known structural
//! weaknesses for secret-sharing or erasure applications, and is the
//! conventional choice across the `RustCrypto` and `reed-solomon` crates.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

/// Reduction polynomial for the field: x^8 + x^4 + x^3 + x + 1.
pub const REDUCTION_POLYNOMIAL: u16 = 0x011B;

/// Addition in GF(2^8): bitwise XOR. Same as subtraction.
#[must_use]
pub fn add(a: u8, b: u8) -> u8 {
    a ^ b
}

/// Multiplication in GF(2^8) via shift-XOR with reduction.
#[must_use]
pub fn mul(a: u8, b: u8) -> u8 {
    let mut a = u16::from(a);
    let mut b = b;
    let mut acc: u16 = 0;
    while b != 0 {
        if b & 1 != 0 {
            acc ^= a;
        }
        b >>= 1;
        let high_bit_set = a & 0x80 != 0;
        a <<= 1;
        if high_bit_set {
            a ^= REDUCTION_POLYNOMIAL;
        }
    }
    // Mask guarantees the low byte is the GF product; truncation is sound.
    u8::try_from(acc & 0xFF).unwrap_or(0)
}

/// Multiplicative inverse via Fermat's little theorem: a^(p-2) = a^-1
/// in GF(p). For GF(2^8), p-2 = 254. The inverse of 0 is defined as 0
/// by convention (callers MUST guard against this if it matters).
#[must_use]
pub fn inv(a: u8) -> u8 {
    if a == 0 {
        return 0;
    }
    pow(a, 254)
}

/// Exponentiation in GF(2^8) via square-and-multiply.
#[must_use]
pub fn pow(mut a: u8, mut exp: u32) -> u8 {
    let mut acc: u8 = 1;
    while exp > 0 {
        if exp & 1 == 1 {
            acc = mul(acc, a);
        }
        a = mul(a, a);
        exp >>= 1;
    }
    acc
}

/// Dot product of two equal-length slices in GF(2^8).
///
/// # Panics
///
/// Panics if the slices have different lengths. Callers MUST
/// pre-validate.
#[must_use]
pub fn dot(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len(), "gf256::dot requires equal lengths");
    let mut acc: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        acc = add(acc, mul(*x, *y));
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_is_xor() {
        assert_eq!(add(0, 0), 0);
        assert_eq!(add(0xFF, 0xFF), 0);
        assert_eq!(add(0x0F, 0xF0), 0xFF);
        assert_eq!(add(0x42, 0x42), 0);
    }

    #[test]
    fn mul_zero_absorbs() {
        assert_eq!(mul(0, 42), 0);
        assert_eq!(mul(42, 0), 0);
        assert_eq!(mul(0, 0), 0);
    }

    #[test]
    fn mul_one_is_identity() {
        for a in 0..=u8::MAX {
            assert_eq!(mul(a, 1), a);
        }
    }

    #[test]
    fn mul_commutative() {
        for a in [0, 1, 2, 3, 5, 7, 53, 200, 255] {
            for b in [0, 1, 2, 3, 5, 7, 53, 200, 255] {
                assert_eq!(mul(a, b), mul(b, a), "a={a} b={b}");
            }
        }
    }

    #[test]
    fn mul_associative_sample() {
        for a in [1, 5, 17, 100, 200] {
            for b in [1, 5, 17, 100, 200] {
                for c in [1, 5, 17, 100, 200] {
                    let ab_c = mul(mul(a, b), c);
                    let a_bc = mul(a, mul(b, c));
                    assert_eq!(ab_c, a_bc, "a={a} b={b} c={c}");
                }
            }
        }
    }

    #[test]
    fn inv_round_trips() {
        for a in 1..=u8::MAX {
            let inv_a = inv(a);
            assert_ne!(inv_a, 0, "a={a} has no inverse");
            assert_eq!(mul(a, inv_a), 1, "a={a} * inv(a) != 1");
        }
    }

    #[test]
    fn inv_zero_is_zero_by_convention() {
        assert_eq!(inv(0), 0);
    }

    #[test]
    fn pow_zero_is_one() {
        assert_eq!(pow(5, 0), 1);
        assert_eq!(pow(255, 0), 1);
    }

    #[test]
    fn pow_one_is_identity() {
        for a in 0..=u8::MAX {
            assert_eq!(pow(a, 1), a);
        }
    }

    #[test]
    fn pow_2_is_square() {
        for a in [0, 1, 2, 5, 17, 53, 100, 200, 255] {
            assert_eq!(pow(a, 2), mul(a, a));
        }
    }

    #[test]
    fn dot_self_vs_known() {
        // dot([1,2,3], [1,2,3]) = 1*1 + 2*2 + 3*3 (in GF)
        let v = [1u8, 2, 3];
        let expected = add(add(mul(1, 1), mul(2, 2)), mul(3, 3));
        assert_eq!(dot(&v, &v), expected);
    }

    #[test]
    fn dot_zero_vector() {
        assert_eq!(dot(&[0, 0, 0], &[1, 2, 3]), 0);
        assert_eq!(dot(&[], &[]), 0);
    }

    #[test]
    #[should_panic(expected = "requires equal lengths")]
    fn dot_panics_on_mismatched_lengths() {
        let _ = dot(&[1, 2], &[3]);
    }

    #[test]
    fn fermat_little_theorem_holds() {
        // For nonzero a in GF(2^8): a^255 = 1
        for a in [1, 5, 17, 53, 100, 200, 255] {
            assert_eq!(pow(a, 255), 1, "a={a}");
        }
    }
}
