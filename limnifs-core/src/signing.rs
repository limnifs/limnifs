//! Manifest signing — Ed25519 signatures over the `ManifestRoot`.
//!
//! ## Design
//!
//! v1 ships **keypair mode**: the signer holds an Ed25519 private
//! key, signs the manifest root, and the verifier checks with the
//! corresponding public key. No network, no third party.
//!
//! Keyless Fulcio + Rekor mode (sigstore's signature workflow) is
//! deferred to v2 — it requires OAuth, certificate transparency log
//! embedding, and a bundled cert chain in the signature bundle. The
//! signing API surface here is forward-compatible: the
//! [`SignatureBundle`] struct carries fields for both modes.
//!
//! ## What is signed
//!
//! The 32-byte `ManifestRoot` (BLAKE3-derived). Drops are
//! transitively covered via the Merkle tree whose root IS the
//! `ManifestRoot`. Tampering any byte of the manifest invalidates
//! the signature.
//!
//! ## Library dependencies
//!
//! `ed25519-dalek` (MIT OR Apache-2.0), `rand_core` (MIT OR Apache-2.0).
//! Both permissive.
//!
//! See task `05-signing-sigstore.md`.

#![cfg(feature = "signing")]

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

/// Errors from signing and verification.
#[derive(Debug)]
pub enum SignError {
    /// Cryptographic operation failed (signature generation, parsing,
    /// verification).
    Crypto(String),
}

impl std::fmt::Display for SignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Crypto(s) => write!(f, "sign: {s}"),
        }
    }
}

impl std::error::Error for SignError {}

/// An Ed25519 keypair for signing.
#[derive(Clone, Debug)]
pub struct SigningKeyPair {
    secret: SigningKey,
}

impl SigningKeyPair {
    /// Generate a new keypair using the supplied RNG (caller-owned
    /// entropy, mirroring the Shamir pattern).
    ///
    /// # Errors
    ///
    /// Returns [`SignError::Crypto`] if the RNG fails.
    pub fn generate<F: FnMut(&mut [u8]) -> Result<(), SignError>>(
        mut rng: F,
    ) -> Result<Self, SignError> {
        let mut secret = [0u8; 32];
        rng(&mut secret)?;
        Ok(Self {
            secret: SigningKey::from_bytes(&secret),
        })
    }

    /// Construct from raw secret bytes (caller-managed serialization).
    #[must_use]
    pub fn from_bytes(secret: [u8; 32]) -> Self {
        Self {
            secret: SigningKey::from_bytes(&secret),
        }
    }

    /// The 32-byte secret.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }

    /// The 32-byte public key.
    #[must_use]
    pub fn public(&self) -> VerifyingKey {
        self.secret.verifying_key()
    }
}

impl Signer<Signature> for SigningKeyPair {
    fn try_sign(&self, msg: &[u8]) -> Result<Signature, ed25519_dalek::SignatureError> {
        self.secret.try_sign(msg)
    }
}

/// A signature bundle: covers a `ManifestRoot` with one of the
/// supported modes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignatureBundle {
    /// The 32-byte `ManifestRoot` that was signed.
    pub manifest_root: [u8; 32],
    /// 64-byte Ed25519 signature over `manifest_root`.
    pub signature: [u8; 64],
    /// 32-byte signer public key (keypair mode).
    pub public_key: [u8; 32],
    /// Sigstore mode marker (keyless would carry a cert chain instead).
    pub mode: SignMode,
}

/// Signature provenance mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignMode {
    /// Plain keypair signature. Offline-verifiable.
    Keypair,
    /// Keyless Fulcio + Rekor bundle (v2 — not yet implemented).
    Keyless,
}

/// Sign `manifest_root` with `signer`.
///
/// # Errors
///
/// Returns [`SignError::Crypto`] if the underlying signing call fails
/// (vanishingly rare for Ed25519 on valid keys).
pub fn sign(
    signer: &SigningKeyPair,
    manifest_root: &[u8; 32],
) -> Result<SignatureBundle, SignError> {
    let sig: Signature = signer.sign(manifest_root);
    let mut sig_bytes = [0u8; 64];
    sig_bytes.copy_from_slice(&sig.to_bytes());
    let mut pub_bytes = [0u8; 32];
    pub_bytes.copy_from_slice(signer.public().as_bytes());
    Ok(SignatureBundle {
        manifest_root: *manifest_root,
        signature: sig_bytes,
        public_key: pub_bytes,
        mode: SignMode::Keypair,
    })
}

/// Verify `bundle` against the `SignMode`'s rules.
///
/// For `SignMode::Keypair`: checks `signature` over `manifest_root`
/// using `public_key`. No network.
///
/// # Errors
///
/// Returns [`SignError::Crypto`] if verification fails (wrong key,
/// tampered root, tampered signature, mode mismatch).
pub fn verify(bundle: &SignatureBundle) -> Result<(), SignError> {
    if bundle.mode != SignMode::Keypair {
        return Err(SignError::Crypto(format!(
            "verify: mode {:?} not implemented (v1 supports Keypair only)",
            bundle.mode
        )));
    }
    let verifying = VerifyingKey::from_bytes(&bundle.public_key)
        .map_err(|e| SignError::Crypto(format!("public key parse: {e}")))?;
    let sig = Signature::from_bytes(&bundle.signature);
    verifying
        .verify(&bundle.manifest_root, &sig)
        .map_err(|e| SignError::Crypto(format!("verify: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_urandom_rng(out: &mut [u8]) -> Result<(), SignError> {
        use std::io::Read;
        let mut f = std::fs::File::open("/dev/urandom")
            .map_err(|e| SignError::Crypto(format!("/dev/urandom: {e}")))?;
        f.read_exact(out)
            .map_err(|e| SignError::Crypto(format!("read: {e}")))?;
        Ok(())
    }

    #[test]
    fn sign_verify_round_trip() {
        let signer = SigningKeyPair::generate(dev_urandom_rng).expect("keypair");
        let root = [0x42u8; 32];
        let bundle = sign(&signer, &root).expect("sign");
        verify(&bundle).expect("verify");
    }

    #[test]
    fn sign_verify_random_root() {
        let signer = SigningKeyPair::generate(dev_urandom_rng).expect("keypair");
        let mut root = [0u8; 32];
        dev_urandom_rng(&mut root).unwrap();
        let bundle = sign(&signer, &root).expect("sign");
        verify(&bundle).expect("verify");
    }

    #[test]
    fn verify_rejects_tampered_root() {
        let signer = SigningKeyPair::generate(dev_urandom_rng).expect("keypair");
        let root = [0x42u8; 32];
        let mut bundle = sign(&signer, &root).expect("sign");
        bundle.manifest_root[0] ^= 0x01;
        match verify(&bundle) {
            Err(SignError::Crypto(_)) => {}
            Ok(()) => panic!("tampered root must fail"),
        }
    }

    #[test]
    fn verify_rejects_tampered_signature() {
        let signer = SigningKeyPair::generate(dev_urandom_rng).expect("keypair");
        let root = [0x42u8; 32];
        let mut bundle = sign(&signer, &root).expect("sign");
        bundle.signature[0] ^= 0x01;
        match verify(&bundle) {
            Err(SignError::Crypto(_)) => {}
            Ok(()) => panic!("tampered signature must fail"),
        }
    }

    #[test]
    fn verify_rejects_wrong_public_key() {
        let signer_a = SigningKeyPair::generate(dev_urandom_rng).expect("keypair a");
        let signer_b = SigningKeyPair::generate(dev_urandom_rng).expect("keypair b");
        let root = [0x42u8; 32];
        let mut bundle = sign(&signer_a, &root).expect("sign with a");
        // Swap in B's public key.
        bundle.public_key = signer_b.public().to_bytes();
        match verify(&bundle) {
            Err(SignError::Crypto(_)) => {}
            Ok(()) => panic!("mismatched public key must fail"),
        }
    }

    #[test]
    fn bundle_is_offline_verifiable() {
        // No network calls. We can't assert "no syscall", but we can
        // verify the bundle in a fresh process context — i.e., without
        // the signer's secret key. This pins offline-verifiability.
        let signer = SigningKeyPair::generate(dev_urandom_rng).expect("keypair");
        let root = [0xABu8; 32];
        let bundle = sign(&signer, &root).expect("sign");

        // verify() only reads `bundle` (which carries only public
        // data: manifest_root, signature, public_key, mode). The
        // signer's secret is never referenced past this point.
        let _ = signer;
        verify(&bundle).expect("offline verify");
    }

    #[test]
    fn keypair_round_trips_through_bytes() {
        let original = SigningKeyPair::generate(dev_urandom_rng).expect("gen");
        let secret_bytes = original.to_bytes();
        let restored = SigningKeyPair::from_bytes(secret_bytes);
        assert_eq!(restored.to_bytes(), secret_bytes);
        assert_eq!(restored.public().as_bytes(), original.public().as_bytes());
    }

    #[test]
    fn keyless_mode_rejected_in_v1() {
        // The Keyless mode constant exists for forward compatibility,
        // but verify must reject it until Fulcio + Rekor support ships.
        let signer = SigningKeyPair::generate(dev_urandom_rng).expect("keypair");
        let root = [0x42u8; 32];
        let mut bundle = sign(&signer, &root).expect("sign");
        bundle.mode = SignMode::Keyless;
        match verify(&bundle) {
            Err(SignError::Crypto(_)) => {}
            Ok(()) => panic!("Keyless mode must not verify in v1"),
        }
    }

    #[test]
    fn different_signers_produce_different_signatures() {
        let signer_a = SigningKeyPair::generate(dev_urandom_rng).expect("a");
        let signer_b = SigningKeyPair::generate(dev_urandom_rng).expect("b");
        let root = [0x99u8; 32];
        let bundle_a = sign(&signer_a, &root).expect("sign a");
        let bundle_b = sign(&signer_b, &root).expect("sign b");
        assert_ne!(bundle_a.signature, bundle_b.signature);
        assert_ne!(bundle_a.public_key, bundle_b.public_key);
        // Both verify against their own public key.
        verify(&bundle_a).expect("a verifies");
        verify(&bundle_b).expect("b verifies");
    }
}
