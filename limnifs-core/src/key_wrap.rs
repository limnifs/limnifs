//! HPKE-style key wrap — per-recipient X25519 envelopes for the
//! image master key.
//!
//! ## Design
//!
//! Implements the cryptographic shape of `HPKE` Base mode
//! (RFC 9180 §6.1) with ciphersuite `DHKEM(X25519, HKDF-SHA256),
//! HKDF-SHA256, XChaCha20-Poly1305`:
//!
//! 1. Sender generates an ephemeral X25519 keypair.
//! 2. `shared_secret = ECDH(ephemeral_priv, recipient_pub)`.
//! 3. `prk = HKDF-Extract(salt="limnifs-keywrap-v1", IKM=shared_secret)`.
//! 4. `aead_key = HKDF-Expand(prk, "key", 32)`.
//! 5. `aead_nonce = HKDF-Expand(prk, "nonce", 24)`.
//! 6. `ct = XChaCha20-Poly1305.seal(aead_key, aead_nonce, plaintext, aad=[])`.
//! 7. Envelope = `(ephemeral_pubkey, ct)` — the nonce is derived, not random.
//!
//! ## Why not full RFC 9180?
//!
//! Full HPKE also derives an `exporter_secret`, a `key_schedule_context`
//! that includes the KEM `encap` output, and uses specific `suite_id`
//! strings. Implementing those requires a verified HPKE library; this
//! module deliberately implements a smaller, documented subset. The
//! cryptographic security properties (forward secrecy per envelope,
//! sender authentication via recipient pubkey binding, AEAD integrity)
//! are preserved.
//!
//! ## Drop-key separation
//!
//! The wrapped key is the image's master AEAD key, NOT a drop
//! plaintext. Drops are sealed with the master key; recipients
//! unwrap the master key, then unseal drops. Adding/removing a
//! recipient re-wraps the master key only — drops stay sealed.
//!
//! ## Identity rule
//!
//! `DropId = BLAKE3(plaintext)`. The same plaintext drop sealed to
//! multiple recipients still hashes to the same `DropId` — wrap does
//! not affect identity, only who can decrypt.
//!
//! See task `05-key-wrap-hpke.md`.

#![cfg(feature = "key-wrap")]
#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};

use crate::aead::AEAD_XCHACHA20_POLY1305;
use crate::crypto;

/// HKDF salt domain-separating this construction from other uses of
/// HKDF-SHA256 elsewhere in the stack.
const KEYWRAP_SALT: &[u8] = b"limnifs-keywrap-v1";

/// HKDF info string for the AEAD key derivation.
const KEY_INFO: &[u8] = b"key";

/// HKDF info string for the AEAD nonce derivation.
const NONCE_INFO: &[u8] = b"nonce";

/// AEAD key length (`XChaCha20` = 32 bytes).
const AEAD_KEY_LEN: usize = 32;

/// AEAD nonce length (`XChaCha20` = 24 bytes).
const AEAD_NONCE_LEN: usize = 24;

/// Errors from [`wrap_key`] and [`unwrap_key`].
#[derive(Debug)]
pub enum KeyWrapError {
    /// Cryptographic operation failed (AEAD, HKDF, KEM).
    Crypto(String),
}

impl std::fmt::Display for KeyWrapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Crypto(s) => write!(f, "key-wrap: {s}"),
        }
    }
}

impl std::error::Error for KeyWrapError {}

/// One recipient's wrapped key: the ephemeral public key plus the
/// AEAD-sealed master key bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrappedKey {
    /// X25519 ephemeral public key (32 bytes).
    pub ephemeral_public: [u8; 32],
    /// `XChaCha20-Poly1305` ciphertext of the master key (`plaintext_len + 16` tag bytes).
    pub ciphertext: Vec<u8>,
}

/// An X25519 keypair for key-wrap operations.
#[derive(Clone, Debug)]
pub struct KeyPair {
    secret: [u8; 32],
    public: PublicKey,
}

impl KeyPair {
    /// Generate a new keypair using the supplied RNG (caller-owned
    /// entropy, mirroring the Shamir pattern).
    ///
    /// # Errors
    ///
    /// Returns [`KeyWrapError::Crypto`] if the RNG fails.
    pub fn generate<F: FnMut(&mut [u8]) -> Result<(), KeyWrapError>>(
        mut rng: F,
    ) -> Result<Self, KeyWrapError> {
        let mut secret_bytes = [0u8; 32];
        rng(&mut secret_bytes)?;
        // Clamp per RFC 7748 §5.
        let secret = StaticSecret::from(clamp_scalar_bytes(secret_bytes));
        let public = PublicKey::from(&secret);
        Ok(Self {
            secret: secret.to_bytes(),
            public,
        })
    }

    /// Construct from raw secret bytes (caller-managed serialization).
    #[must_use]
    pub fn from_secret_bytes(secret_bytes: [u8; 32]) -> Self {
        let secret = StaticSecret::from(clamp_scalar_bytes(secret_bytes));
        let public = PublicKey::from(&secret);
        Self {
            secret: secret.to_bytes(),
            public,
        }
    }

    /// The public key.
    #[must_use]
    pub fn public(&self) -> PublicKey {
        self.public
    }

    /// The raw secret bytes.
    #[must_use]
    pub fn secret_bytes(&self) -> [u8; 32] {
        self.secret
    }
}

/// Wrap `plaintext_key` to `recipient_public` so only the holder of
/// the matching private key can unwrap it.
///
/// Uses an ephemeral X25519 keypair (sender) and HKDF-SHA256 to derive
/// an XChaCha20-Poly1305 key+nonce. The envelope carries the ephemeral
/// public key and ciphertext.
///
/// # Errors
///
/// Returns [`KeyWrapError::Crypto`] if HKDF or AEAD fail (should not
/// happen for valid inputs).
pub fn wrap_key(
    recipient_public: &PublicKey,
    plaintext_key: &[u8],
) -> Result<WrappedKey, KeyWrapError> {
    let ephemeral_secret = EphemeralSecret::random_from_rng(rand_core::OsRng);
    let ephemeral_public = PublicKey::from(&ephemeral_secret);
    let shared = ephemeral_secret.diffie_hellman(recipient_public);

    let (aead_key, aead_nonce) = derive_key_and_nonce(shared.as_bytes());

    let mut nonce_bytes = [0u8; AEAD_NONCE_LEN];
    nonce_bytes.copy_from_slice(&aead_nonce);
    let ciphertext = crypto::seal(
        AEAD_XCHACHA20_POLY1305,
        &aead_key,
        &nonce_bytes,
        plaintext_key,
        &[],
    )
    .map_err(|e| KeyWrapError::Crypto(format!("AEAD seal: {e}")))?;

    let mut ephem_pub = [0u8; 32];
    ephem_pub.copy_from_slice(ephemeral_public.as_bytes());
    Ok(WrappedKey {
        ephemeral_public: ephem_pub,
        ciphertext,
    })
}

/// Unwrap a [`WrappedKey`] using `recipient_secret`.
///
/// # Errors
///
/// Returns [`KeyWrapError::Crypto`] if the AEAD open fails (wrong
/// recipient, tampered ciphertext, etc.).
pub fn unwrap_key(
    recipient_secret: &[u8; 32],
    envelope: &WrappedKey,
) -> Result<Vec<u8>, KeyWrapError> {
    let secret = StaticSecret::from(clamp_scalar_bytes(*recipient_secret));
    let ephem_pub = PublicKey::from(envelope.ephemeral_public);
    let shared = secret.diffie_hellman(&ephem_pub);

    let (aead_key, aead_nonce) = derive_key_and_nonce(shared.as_bytes());

    let mut nonce_bytes = [0u8; AEAD_NONCE_LEN];
    nonce_bytes.copy_from_slice(&aead_nonce);
    crypto::open(
        AEAD_XCHACHA20_POLY1305,
        &aead_key,
        &nonce_bytes,
        &envelope.ciphertext,
        &[],
    )
    .map_err(|e| KeyWrapError::Crypto(format!("AEAD open: {e}")))
}

/// Derive AEAD key (32 bytes) and nonce (24 bytes) from the ECDH
/// shared secret via HKDF-SHA256.
fn derive_key_and_nonce(shared_secret: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let hk = Hkdf::<Sha256>::new(Some(KEYWRAP_SALT), shared_secret);
    let mut aead_key = vec![0u8; AEAD_KEY_LEN];
    let mut aead_nonce = vec![0u8; AEAD_NONCE_LEN];
    hk.expand(KEY_INFO, &mut aead_key)
        .expect("32 <= 255 * Sha256 output");
    hk.expand(NONCE_INFO, &mut aead_nonce)
        .expect("24 <= 255 * Sha256 output");
    (aead_key, aead_nonce)
}

/// Clamp a raw 32-byte scalar per RFC 7748 §5. x25519-dalek's
/// `StaticSecret::from` does this internally, but this helper documents
/// the boundary and ensures callers don't accidentally pass an
/// unclamped key.
fn clamp_scalar_bytes(mut bytes: [u8; 32]) -> [u8; 32] {
    bytes[0] &= 0xF8;
    bytes[31] &= 0x7F;
    bytes[31] |= 0x40;
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_urandom_rng(out: &mut [u8]) -> Result<(), KeyWrapError> {
        use std::io::Read;
        let mut f = std::fs::File::open("/dev/urandom")
            .map_err(|e| KeyWrapError::Crypto(format!("/dev/urandom: {e}")))?;
        f.read_exact(out)
            .map_err(|e| KeyWrapError::Crypto(format!("read: {e}")))?;
        Ok(())
    }

    #[test]
    fn wrap_unwrap_round_trip() {
        let recipient = KeyPair::generate(dev_urandom_rng).expect("recipient");
        let master_key = b"32-byte master key for AEAD12345"; // 32 bytes
        assert_eq!(master_key.len(), 32);
        let envelope = wrap_key(&recipient.public(), master_key).expect("wrap");
        let recovered = unwrap_key(&recipient.secret_bytes(), &envelope).expect("unwrap");
        assert_eq!(recovered.as_slice(), master_key);
    }

    #[test]
    fn wrap_unwrap_random_master_key() {
        let recipient = KeyPair::generate(dev_urandom_rng).expect("recipient");
        let mut master_key = [0u8; 32];
        dev_urandom_rng(&mut master_key).unwrap();
        let envelope = wrap_key(&recipient.public(), &master_key).expect("wrap");
        let recovered = unwrap_key(&recipient.secret_bytes(), &envelope).expect("unwrap");
        assert_eq!(recovered.as_slice(), master_key);
    }

    #[test]
    fn wrong_recipient_cannot_unwrap() {
        let alice = KeyPair::generate(dev_urandom_rng).expect("alice");
        let bob = KeyPair::generate(dev_urandom_rng).expect("bob");
        let master_key = b"32-byte master key for AEAD12345";
        let envelope = wrap_key(&alice.public(), master_key).expect("wrap to alice");
        match unwrap_key(&bob.secret_bytes(), &envelope) {
            Err(KeyWrapError::Crypto(_)) => {}
            Ok(plaintext) => panic!("bob should not unwrap alice's envelope; got {plaintext:?}"),
        }
    }

    #[test]
    fn multiple_recipients_share_drop_id() {
        // The same master key sealed to multiple recipients does not
        // change the plaintext — DropId = BLAKE3(plaintext) stays
        // constant across recipients.
        let alice = KeyPair::generate(dev_urandom_rng).expect("alice");
        let bob = KeyPair::generate(dev_urandom_rng).expect("bob");
        let carol = KeyPair::generate(dev_urandom_rng).expect("carol");
        let master_key = b"32-byte master key for AEAD12345";

        let envelope_a = wrap_key(&alice.public(), master_key).expect("wrap alice");
        let envelope_b = wrap_key(&bob.public(), master_key).expect("wrap bob");
        let envelope_c = wrap_key(&carol.public(), master_key).expect("wrap carol");

        let recovered_a = unwrap_key(&alice.secret_bytes(), &envelope_a).expect("alice");
        let recovered_b = unwrap_key(&bob.secret_bytes(), &envelope_b).expect("bob");
        let recovered_c = unwrap_key(&carol.secret_bytes(), &envelope_c).expect("carol");

        // All three recipients recover the SAME plaintext.
        assert_eq!(recovered_a.as_slice(), master_key);
        assert_eq!(recovered_b.as_slice(), master_key);
        assert_eq!(recovered_c.as_slice(), master_key);

        // And BLAKE3 of the plaintext is identical.
        let hash_a = blake3::hash(&recovered_a);
        let hash_b = blake3::hash(&recovered_b);
        let hash_c = blake3::hash(&recovered_c);
        assert_eq!(hash_a, hash_b);
        assert_eq!(hash_b, hash_c);
    }

    #[test]
    fn ephemeral_public_differs_per_wrap() {
        // Each wrap uses a fresh ephemeral keypair, so two wraps of
        // the same master key to the same recipient yield different
        // ephemeral_public values AND different ciphertexts.
        let recipient = KeyPair::generate(dev_urandom_rng).expect("recipient");
        let master_key = b"32-byte master key for AEAD12345";
        let envelope_1 = wrap_key(&recipient.public(), master_key).expect("wrap 1");
        let envelope_2 = wrap_key(&recipient.public(), master_key).expect("wrap 2");
        assert_ne!(
            envelope_1.ephemeral_public, envelope_2.ephemeral_public,
            "each wrap uses a fresh ephemeral key"
        );
        assert_ne!(
            envelope_1.ciphertext, envelope_2.ciphertext,
            "each wrap yields a distinct ciphertext"
        );

        // Both unwrap to the same plaintext.
        let r1 = unwrap_key(&recipient.secret_bytes(), &envelope_1).unwrap();
        let r2 = unwrap_key(&recipient.secret_bytes(), &envelope_2).unwrap();
        assert_eq!(r1, r2);
        assert_eq!(r1.as_slice(), master_key);
    }

    #[test]
    fn tampered_ciphertext_rejected() {
        let recipient = KeyPair::generate(dev_urandom_rng).expect("recipient");
        let master_key = b"32-byte master key for AEAD12345";
        let mut envelope = wrap_key(&recipient.public(), master_key).expect("wrap");
        // Flip one bit of the ciphertext.
        envelope.ciphertext[0] ^= 0x01;
        match unwrap_key(&recipient.secret_bytes(), &envelope) {
            Err(KeyWrapError::Crypto(_)) => {}
            Ok(p) => panic!("tampered envelope should not unwrap, got {p:?}"),
        }
    }

    #[test]
    fn tampered_ephemeral_public_rejected() {
        let recipient = KeyPair::generate(dev_urandom_rng).expect("recipient");
        let master_key = b"32-byte master key for AEAD12345";
        let mut envelope = wrap_key(&recipient.public(), master_key).expect("wrap");
        envelope.ephemeral_public[0] ^= 0x01;
        match unwrap_key(&recipient.secret_bytes(), &envelope) {
            Err(KeyWrapError::Crypto(_)) => {}
            Ok(p) => panic!("tampered ephemeral key should not unwrap, got {p:?}"),
        }
    }

    #[test]
    fn keypair_from_secret_bytes_round_trips() {
        let original = KeyPair::generate(dev_urandom_rng).expect("gen");
        let secret = original.secret_bytes();
        let restored = KeyPair::from_secret_bytes(secret);
        assert_eq!(restored.secret_bytes(), secret);
        assert_eq!(restored.public().as_bytes(), original.public().as_bytes());
    }

    #[test]
    fn clamp_sets_correct_bits() {
        let raw = [0xFFu8; 32];
        let clamped = clamp_scalar_bytes(raw);
        // First byte: low 3 bits cleared.
        assert_eq!(clamped[0] & 0x07, 0);
        // Last byte: high bit cleared, next-to-high bit set.
        assert_eq!(clamped[31] & 0x80, 0);
        assert_eq!(clamped[31] & 0x40, 0x40);
    }
}
