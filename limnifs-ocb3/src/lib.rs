//! `limnifs-ocb3` — Pure-Rust OCB3 AEAD (RFC 7253).
//!
//! A standalone implementation of the Offset Codebook Mode v3
//! authenticated encryption, built on the stable `aes 0.8` crate.
//! No release-candidate dependencies.
//!
//! ## Why this crate exists
//!
//! The `ocb3` crate from RustCrypto depends on RC versions of
//! `cipher 0.5` / `aes 0.9` that conflict with the stable `aes-gcm`
//! and `chacha20poly1305` crates. This crate implements the same
//! algorithm using only the stable `aes 0.8` / `cipher 0.4`
//! ecosystem.
//!
//! ## API
//!
//! ```no_run
//! use limnifs_ocb3::Ocb3Aes256;
//!
//! let key = [0x42u8; 32];
//! let nonce = [0xABu8; 12];
//! let aad = b"associated data";
//! let plaintext = b"secret message";
//!
//! let ocb = Ocb3Aes256::new(&key);
//! let mut buffer = plaintext.to_vec();
//! let tag = ocb.encrypt_in_place_detached(&nonce, aad, &mut buffer);
//!
//! // buffer now holds ciphertext; tag is the 16-byte auth tag.
//! ocb.decrypt_in_place_detached(&nonce, aad, &mut buffer, &tag)
//!     .expect("tag mismatch");
//! // buffer now holds the original plaintext.
//! ```
//!
//! ## Algorithm
//!
//! OCB3 encrypts each plaintext block as `C_i = E_K(P_i ⊕ Offset_i) ⊕ Offset_i`,
//! where `Offset_i` is derived from a nonce and key-dependent L-values.
//! The authentication tag is computed from a plaintext checksum and an
//! associated-data hash. See [RFC 7253](https://www.rfc-editor.org/rfc/rfc7253.html).

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

use aes::cipher::{generic_array::GenericArray, BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes256;

/// AES block size in bytes.
const BLOCK_SIZE: usize = 16;
/// OCB3 nonce size in bytes (96 bits, the recommended size).
pub const NONCE_SIZE: usize = 12;
/// OCB3 authentication tag size in bytes (128 bits).
pub const TAG_SIZE: usize = 16;
/// GF(2^128) reduction polynomial for AES (little-endian constant).
const GF128_POLY: u8 = 0x87;
/// Maximum number of precomputed L-values. Supports plaintext up to
/// 2^(L_TABLE_SIZE + 4) bytes ≈ 1 GiB.
const L_TABLE_SIZE: usize = 24;

type Block = [u8; BLOCK_SIZE];
type Tag = [u8; TAG_SIZE];

/// OCB3 authenticated encryption using AES-256.
///
/// Construct with [`Ocb3Aes256::new`] from a 32-byte key.
pub struct Ocb3Aes256 {
    cipher: Aes256,
    /// L* = dbl(E_K(0)).
    ll_star: Block,
    /// L$ = dbl(L*).
    ll_dollar: Block,
    /// Precomputed L[i] = dbl^(i+1)(L*), for offset computation.
    ll: [Block; L_TABLE_SIZE],
}

impl Ocb3Aes256 {
    /// Create a new OCB3 instance from a 32-byte AES-256 key.
    #[must_use]
    pub fn new(key: &[u8; 32]) -> Self {
        let cipher = Aes256::new(GenericArray::from_slice(key));
        let (ll_star, ll_dollar, ll) = Self::derive_key_variables(&cipher);
        Self {
            cipher,
            ll_star,
            ll_dollar,
            ll,
        }
    }

    /// Encrypt `buffer` in place and return the authentication tag.
    ///
    /// After calling, `buffer` contains the ciphertext. The returned
    /// 16-byte tag authenticates both the ciphertext and `aad`.
    #[must_use]
    pub fn encrypt_in_place_detached(
        &self,
        nonce: &[u8; NONCE_SIZE],
        aad: &[u8],
        buffer: &mut [u8],
    ) -> Tag {
        let (bottom, stretch) = self.derive_nonce_variables(nonce);
        let mut offset = Self::initial_offset(bottom, &stretch);

        let mut checksum: Block = [0; BLOCK_SIZE];
        let full_blocks = buffer.len() / BLOCK_SIZE;
        let remainder = buffer.len() % BLOCK_SIZE;

        // Process full 16-byte blocks.
        for i in 0..full_blocks {
            let block_start = i * BLOCK_SIZE;
            xor_block(&mut offset, &self.ll[ntz(i + 1)]);
            xor_block_slice(&mut checksum, &buffer[block_start..block_start + BLOCK_SIZE]);
            xor_block_slice(&mut buffer[block_start..block_start + BLOCK_SIZE], &offset);
            self.encrypt_block(&mut buffer[block_start..block_start + BLOCK_SIZE]);
            xor_block_slice(&mut buffer[block_start..block_start + BLOCK_SIZE], &offset);
        }

        // Process final partial block (if any).
        if remainder > 0 {
            let last_start = full_blocks * BLOCK_SIZE;
            xor_block(&mut offset, &self.ll_star);

            let mut pad: Block = offset;
            self.encrypt_block(&mut pad);

            // Checksum: pad plaintext with 0x80 then zeros.
            let mut padded: Block = [0; BLOCK_SIZE];
            padded[..remainder].copy_from_slice(&buffer[last_start..last_start + remainder]);
            padded[remainder] = 0x80;
            xor_block(&mut checksum, &padded);

            // C_* = P_* XOR Pad[0..remainder]
            for j in 0..remainder {
                buffer[last_start + j] ^= pad[j];
            }
        }

        self.compute_tag(aad, &mut checksum, &offset)
    }

    /// Decrypt `buffer` in place, verifying the authentication tag.
    ///
    /// Returns `Ok(())` if the tag is valid (buffer now contains
    /// plaintext) or `Err(())` on tag mismatch (buffer is zeroed).
    ///
    /// # Errors
    /// Returns `Err(())` if the computed tag does not match `tag`.
    pub fn decrypt_in_place_detached(
        &self,
        nonce: &[u8; NONCE_SIZE],
        aad: &[u8],
        buffer: &mut [u8],
        tag: &Tag,
    ) -> Result<(), ()> {
        let (bottom, stretch) = self.derive_nonce_variables(nonce);
        let mut offset = Self::initial_offset(bottom, &stretch);

        let mut checksum: Block = [0; BLOCK_SIZE];
        let full_blocks = buffer.len() / BLOCK_SIZE;
        let remainder = buffer.len() % BLOCK_SIZE;

        // Decrypt full blocks.
        for i in 0..full_blocks {
            let block_start = i * BLOCK_SIZE;
            xor_block(&mut offset, &self.ll[ntz(i + 1)]);
            xor_block_slice(&mut buffer[block_start..block_start + BLOCK_SIZE], &offset);
            self.decrypt_block(&mut buffer[block_start..block_start + BLOCK_SIZE]);
            xor_block_slice(&mut buffer[block_start..block_start + BLOCK_SIZE], &offset);
            xor_block_slice(&mut checksum, &buffer[block_start..block_start + BLOCK_SIZE]);
        }

        // Decrypt final partial block.
        if remainder > 0 {
            let last_start = full_blocks * BLOCK_SIZE;
            xor_block(&mut offset, &self.ll_star);

            let mut pad: Block = offset;
            self.encrypt_block(&mut pad);

            // P_* = C_* XOR Pad[0..remainder]
            for j in 0..remainder {
                buffer[last_start + j] ^= pad[j];
            }

            let mut padded: Block = [0; BLOCK_SIZE];
            padded[..remainder].copy_from_slice(&buffer[last_start..last_start + remainder]);
            padded[remainder] = 0x80;
            xor_block(&mut checksum, &padded);
        }

        let expected_tag = self.compute_tag(aad, &mut checksum, &offset);

        if constant_time_eq(&expected_tag, tag) {
            Ok(())
        } else {
            // Zero the buffer on failure to prevent partial plaintext leakage.
            for byte in buffer.iter_mut() {
                *byte = 0;
            }
            Err(())
        }
    }

    // ── Internal helpers ───────────────────────────────────────

    fn encrypt_block(&self, block: &mut [u8]) {
        self.cipher
            .encrypt_block(GenericArray::from_mut_slice(&mut block[..BLOCK_SIZE]));
    }

    fn decrypt_block(&self, block: &mut [u8]) {
        self.cipher
            .decrypt_block(GenericArray::from_mut_slice(&mut block[..BLOCK_SIZE]));
    }

    fn derive_key_variables(cipher: &Aes256) -> (Block, Block, [Block; L_TABLE_SIZE]) {
        let mut ll_star: Block = [0; BLOCK_SIZE];
        cipher.encrypt_block(GenericArray::from_mut_slice(&mut ll_star));

        let ll_dollar = dbl(&ll_star);

        let mut ll = [[0u8; BLOCK_SIZE]; L_TABLE_SIZE];
        let mut current = ll_dollar;
        for entry in &mut ll {
            current = dbl(&current);
            *entry = current;
        }

        (ll_star, ll_dollar, ll)
    }

    fn derive_nonce_variables(&self, nonce: &[u8; NONCE_SIZE]) -> (usize, [u8; 24]) {
        // Build the 16-byte nonce block per RFC 7253 §4.2:
        //   num2str(TAGLEN mod 128, 7) || zeros(120 - 8*NONCE_SIZE) || 1 || N
        let mut nonce_block: Block = [0; BLOCK_SIZE];
        // TAGLEN = 16 bytes = 128 bits. 128 % 128 = 0, so the first 7 bits are 0.
        nonce_block[0] = ((TAG_SIZE * 8) % 128) as u8;

        let start = BLOCK_SIZE - NONCE_SIZE;
        nonce_block[start..BLOCK_SIZE].copy_from_slice(nonce);
        nonce_block[start - 1] |= 1;

        let bottom = (nonce_block[15] & 0x3F) as usize;

        // Top = nonce_block with bottom 6 bits cleared.
        let mut top = nonce_block;
        top[15] &= 0xC0;

        self.encrypt_block(&mut top);
        let ktop = top;

        // Stretch = Ktop || (Ktop[0..8] XOR Ktop[1..9])
        let mut stretch = [0u8; 24];
        stretch[..16].copy_from_slice(&ktop);
        for i in 0..8 {
            stretch[16 + i] = ktop[i] ^ ktop[i + 1];
        }

        (bottom, stretch)
    }

    fn initial_offset(bottom: usize, stretch: &[u8; 24]) -> Block {
        let stretch_low = u128::from_be_bytes(stretch[..16].try_into().unwrap());
        let stretch_hi = u128::from_be_bytes({
            let mut hi = [0u8; 16];
            hi[..8].copy_from_slice(&stretch[16..24]);
            hi
        });

        let offset = (stretch_low << bottom) | (stretch_hi >> (64 - bottom));
        offset.to_be_bytes()
    }

    /// HASH function for associated data (RFC 7253 §4.1).
    fn hash_ad(&self, aad: &[u8]) -> Block {
        let mut offset: Block = [0; BLOCK_SIZE];
        let mut sum: Block = [0; BLOCK_SIZE];

        let full_blocks = aad.len() / BLOCK_SIZE;
        let remainder = aad.len() % BLOCK_SIZE;

        for i in 0..full_blocks {
            let block_start = i * BLOCK_SIZE;
            xor_block(&mut offset, &self.ll[ntz(i + 1)]);
            let mut block: Block = [0; BLOCK_SIZE];
            block.copy_from_slice(&aad[block_start..block_start + BLOCK_SIZE]);
            xor_block(&mut block, &offset);
            self.encrypt_block(&mut block);
            xor_block(&mut sum, &block);
        }

        if remainder > 0 {
            let last_start = full_blocks * BLOCK_SIZE;
            xor_block(&mut offset, &self.ll_star);

            let mut padded: Block = [0; BLOCK_SIZE];
            padded[..remainder].copy_from_slice(&aad[last_start..last_start + remainder]);
            padded[remainder] = 0x80;
            xor_block(&mut padded, &offset);
            self.encrypt_block(&mut padded);
            xor_block(&mut sum, &padded);
        }

        sum
    }

    /// Compute the final authentication tag.
    fn compute_tag(&self, aad: &[u8], checksum: &mut Block, offset: &Block) -> Tag {
        // Tag = E_K(Checksum ⊕ Offset_m ⊕ L$) ⊕ HASH(A)
        xor_block(checksum, offset);
        xor_block(checksum, &self.ll_dollar);
        self.encrypt_block(checksum);
        let auth = self.hash_ad(aad);
        xor_block(checksum, &auth);
        *checksum
    }
}

// ── GF(2^128) operations ──────────────────────────────────────

/// Multiply a 128-bit block by x in GF(2^128).
/// This is the "double" operation from RFC 7253 §2.
fn dbl(block: &Block) -> Block {
    let carry = block[0] >> 7;
    let mut result = [0u8; BLOCK_SIZE];

    for i in 0..(BLOCK_SIZE - 1) {
        result[i] = (block[i] << 1) | (block[i + 1] >> 7);
    }
    result[BLOCK_SIZE - 1] = block[BLOCK_SIZE - 1] << 1;

    if carry == 1 {
        result[BLOCK_SIZE - 1] ^= GF128_POLY;
    }

    result
}

/// Number of trailing zero bits (RFC 7253 §2).
fn ntz(n: usize) -> usize {
    n.trailing_zeros() as usize
}

/// XOR block `b` into `a` in place.
fn xor_block(a: &mut Block, b: &Block) {
    for (a_byte, b_byte) in a.iter_mut().zip(b.iter()) {
        *a_byte ^= b_byte;
    }
}

/// XOR a 16-byte slice into a Block.
fn xor_block_slice(block: &mut [u8], src: &[u8]) {
    for (b, s) in block.iter_mut().zip(src.iter().take(BLOCK_SIZE)) {
        *b ^= s;
    }
}

/// Constant-time comparison of two 16-byte tags.
fn constant_time_eq(a: &Tag, b: &Tag) -> bool {
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [0x42; 32];
    const NONCE: [u8; 12] = [0xAB; 12];

    #[test]
    fn round_trip_empty() {
        let ocb = Ocb3Aes256::new(&KEY);
        let mut buffer = Vec::new();
        let tag = ocb.encrypt_in_place_detached(&NONCE, b"", &mut buffer);
        assert_eq!(tag.len(), TAG_SIZE);
        ocb.decrypt_in_place_detached(&NONCE, b"", &mut buffer, &tag)
            .expect("round-trip empty");
        assert!(buffer.is_empty());
    }

    #[test]
    fn round_trip_short() {
        let ocb = Ocb3Aes256::new(&KEY);
        let mut buffer = b"hello world".to_vec();
        let tag = ocb.encrypt_in_place_detached(&NONCE, b"aad", &mut buffer);
        ocb.decrypt_in_place_detached(&NONCE, b"aad", &mut buffer, &tag)
            .expect("round-trip short");
        assert_eq!(&buffer, b"hello world");
    }

    #[test]
    fn round_trip_exact_block() {
        let ocb = Ocb3Aes256::new(&KEY);
        let mut buffer = vec![0x55u8; 16];
        let tag = ocb.encrypt_in_place_detached(&NONCE, b"", &mut buffer);
        ocb.decrypt_in_place_detached(&NONCE, b"", &mut buffer, &tag)
            .expect("round-trip 1 block");
        assert_eq!(buffer, vec![0x55u8; 16]);
    }

    #[test]
    fn round_trip_multi_block() {
        let ocb = Ocb3Aes256::new(&KEY);
        let mut buffer = vec![0x42u8; 100]; // 6 full blocks + 4 bytes
        let tag = ocb.encrypt_in_place_detached(&NONCE, b"metadata", &mut buffer);
        ocb.decrypt_in_place_detached(&NONCE, b"metadata", &mut buffer, &tag)
            .expect("round-trip multi-block");
        assert_eq!(buffer, vec![0x42u8; 100]);
    }

    #[test]
    fn round_trip_large() {
        let ocb = Ocb3Aes256::new(&KEY);
        let original: Vec<u8> = (0..10_000u32).flat_map(|i| i.to_le_bytes()).collect();
        let mut buffer = original.clone();
        let tag = ocb.encrypt_in_place_detached(&NONCE, b"", &mut buffer);
        ocb.decrypt_in_place_detached(&NONCE, b"", &mut buffer, &tag)
            .expect("round-trip large");
        assert_eq!(buffer, original);
    }

    #[test]
    fn detects_tampered_ciphertext() {
        let ocb = Ocb3Aes256::new(&KEY);
        let mut buffer = b"sensitive data here".to_vec();
        let tag = ocb.encrypt_in_place_detached(&NONCE, b"", &mut buffer);
        buffer[0] ^= 0xFF;
        assert!(ocb.decrypt_in_place_detached(&NONCE, b"", &mut buffer, &tag).is_err());
    }

    #[test]
    fn detects_tampered_tag() {
        let ocb = Ocb3Aes256::new(&KEY);
        let mut buffer = b"sensitive data here".to_vec();
        let mut tag = ocb.encrypt_in_place_detached(&NONCE, b"", &mut buffer);
        tag[0] ^= 0xFF;
        assert!(ocb.decrypt_in_place_detached(&NONCE, b"", &mut buffer, &tag).is_err());
    }

    #[test]
    fn detects_tampered_aad() {
        let ocb = Ocb3Aes256::new(&KEY);
        let mut buffer = b"sensitive data".to_vec();
        let tag = ocb.encrypt_in_place_detached(&NONCE, b"original aad", &mut buffer);
        assert!(ocb
            .decrypt_in_place_detached(&NONCE, b"tampered aad", &mut buffer, &tag)
            .is_err());
    }

    #[test]
    fn different_nonces_produce_different_ciphertext() {
        let ocb = Ocb3Aes256::new(&KEY);
        let nonce2 = [0xCDu8; 12];

        let mut buf1 = b"same plaintext".to_vec();
        let mut buf2 = b"same plaintext".to_vec();

        ocb.encrypt_in_place_detached(&NONCE, b"", &mut buf1);
        ocb.encrypt_in_place_detached(&nonce2, b"", &mut buf2);

        assert_ne!(buf1, buf2);
    }

    #[test]
    fn dbl_zero_stays_zero() {
        let zero = [0u8; BLOCK_SIZE];
        assert_eq!(dbl(&zero), zero);
    }

    #[test]
    fn dbl_one_becomes_two() {
        let mut one = [0u8; BLOCK_SIZE];
        one[15] = 1;
        let result = dbl(&one);
        assert_eq!(result[15], 2);
    }

    #[test]
    fn dbl_with_cry() {
        // When MSB is 1, dbl should XOR with the polynomial.
        let mut input = [0u8; BLOCK_SIZE];
        input[0] = 0x80; // Only MSB set
        let result = dbl(&input);
        assert_eq!(result[0], 0x00);
        assert_eq!(result[15], GF128_POLY);
    }

    #[test]
    fn ntz_values() {
        assert_eq!(ntz(1), 0);
        assert_eq!(ntz(2), 1);
        assert_eq!(ntz(4), 2);
        assert_eq!(ntz(8), 3);
        assert_eq!(ntz(3), 0);
        assert_eq!(ntz(6), 1);
        assert_eq!(ntz(12), 2);
    }
}
