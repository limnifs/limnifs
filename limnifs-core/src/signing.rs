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

/// Fixed PKCS#8 v2 DER prefix for an Ed25519 private key
/// (RFC 8410): `PrivateKeyInfo` with algorithm `id-Ed25519` and a
/// 32-byte seed. Every Ed25519 PKCS#8 encoding (OpenSSL, cosign,
/// `openssl genpkey`) has exactly this 16-byte prefix before the seed.
const PKCS8_ED25519_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

/// Fixed SPKI DER prefix for an Ed25519 public key (RFC 8410):
/// 12 bytes then the 32-byte key.
const SPKI_ED25519_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

const PEM_PRIVATE_HEADER: &str = "-----BEGIN PRIVATE KEY-----";
const PEM_PRIVATE_FOOTER: &str = "-----END PRIVATE KEY-----";
const PEM_PUBLIC_HEADER: &str = "-----BEGIN PUBLIC KEY-----";
const PEM_PUBLIC_FOOTER: &str = "-----END PUBLIC KEY-----";

fn pem_wrap(label: &str, der: &[u8]) -> String {
    // 16 bytes per base64 line is conventional enough at these sizes
    // (48-byte DER -> 64 chars of base64 -> 4 lines).
    let b64 = {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(der.len().div_ceil(3) * 4);
        for c in der.chunks(3) {
            let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            out.push(TABLE[(n >> 18) as usize & 63] as char);
            out.push(TABLE[(n >> 12) as usize & 63] as char);
            out.push(if c.len() > 1 {
                TABLE[(n >> 6) as usize & 63] as char
            } else {
                '='
            });
            out.push(if c.len() > 2 {
                TABLE[n as usize & 63] as char
            } else {
                '='
            });
        }
        out
    };
    let mut out = format!("-----BEGIN {label}-----\n");
    for c in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(c).unwrap_or(""));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

fn pem_unwrap(
    expected_header: &str,
    expected_footer: &str,
    pem: &str,
) -> Result<Vec<u8>, SignError> {
    let err = |what: &str| SignError::Crypto(format!("pem: {what}"));
    let trimmed = pem.trim();
    if !trimmed.starts_with(expected_header) {
        return Err(err("unexpected PEM label"));
    }
    let body = trimmed
        .strip_prefix(expected_header)
        .and_then(|r| r.strip_suffix(expected_footer))
        .ok_or_else(|| err("missing PEM footer"))?;
    let b64: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = Vec::with_capacity(b64.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for c in b64.chars() {
        let v = match c {
            'A'..='Z' => u32::from(c) - 65,
            'a'..='z' => u32::from(c) - 71,
            '0'..='9' => u32::from(c) + 4,
            '+' => 62,
            '/' => 63,
            '=' => break,
            _ => return Err(err("invalid base64 character")),
        };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Ok(out)
}

/// Encode a 32-byte Ed25519 seed as a PKCS#8 v2 PEM
/// (`-----BEGIN PRIVATE KEY-----`).
#[must_use]
pub fn encode_private_pkcs8_pem(seed: &[u8; 32]) -> String {
    let mut der = Vec::with_capacity(PKCS8_ED25519_PREFIX.len() + 32);
    der.extend_from_slice(&PKCS8_ED25519_PREFIX);
    der.extend_from_slice(seed);
    pem_wrap("PRIVATE KEY", &der)
}

/// Decode a 32-byte Ed25519 seed from a PKCS#8 v2 PEM.
///
/// # Errors
///
/// Returns [`SignError::Crypto`] if the PEM is malformed or the DER
/// is not an Ed25519 `PrivateKeyInfo` with a 32-byte seed.
pub fn decode_private_pkcs8_pem(pem: &str) -> Result<[u8; 32], SignError> {
    let der = pem_unwrap(PEM_PRIVATE_HEADER, PEM_PRIVATE_FOOTER, pem)?;
    if der.len() != PKCS8_ED25519_PREFIX.len() + 32
        || der[..PKCS8_ED25519_PREFIX.len()] != PKCS8_ED25519_PREFIX
    {
        return Err(SignError::Crypto(
            "not an Ed25519 PKCS#8 private key (unexpected DER layout)".into(),
        ));
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&der[PKCS8_ED25519_PREFIX.len()..]);
    Ok(seed)
}

/// Encode a 32-byte Ed25519 public key as an SPKI PEM
/// (`-----BEGIN PUBLIC KEY-----`).
#[must_use]
pub fn encode_public_spki_pem(public: &[u8; 32]) -> String {
    let mut der = Vec::with_capacity(SPKI_ED25519_PREFIX.len() + 32);
    der.extend_from_slice(&SPKI_ED25519_PREFIX);
    der.extend_from_slice(public);
    pem_wrap("PUBLIC KEY", &der)
}

/// Decode a 32-byte Ed25519 public key from an SPKI PEM.
///
/// # Errors
///
/// Returns [`SignError::Crypto`] if the PEM is malformed or the DER
/// is not an Ed25519 `SubjectPublicKeyInfo`.
pub fn decode_public_spki_pem(pem: &str) -> Result<[u8; 32], SignError> {
    let der = pem_unwrap(PEM_PUBLIC_HEADER, PEM_PUBLIC_FOOTER, pem)?;
    if der.len() != SPKI_ED25519_PREFIX.len() + 32
        || der[..SPKI_ED25519_PREFIX.len()] != SPKI_ED25519_PREFIX
    {
        return Err(SignError::Crypto(
            "not an Ed25519 SPKI public key (unexpected DER layout)".into(),
        ));
    }
    let mut public = [0u8; 32];
    public.copy_from_slice(&der[SPKI_ED25519_PREFIX.len()..]);
    Ok(public)
}

/// Magic for the `.limsig` sidecar file.
pub const LIMSIG_MAGIC: [u8; 4] = *b"LMSG";
/// Length of a canonical `.limsig` sidecar: magic + version + mode +
/// root + pubkey + signature.
pub const LIMSIG_LEN: usize = 4 + 1 + 1 + 32 + 32 + 64;

impl SignatureBundle {
    /// Encode as the canonical `.limsig` sidecar layout:
    /// `LMSG | ver u8 | mode u8 | root [32] | pubkey [32] | sig [64]`.
    #[must_use]
    pub fn to_limsig(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(LIMSIG_LEN);
        out.extend_from_slice(&LIMSIG_MAGIC);
        out.push(1);
        out.push(match self.mode {
            SignMode::Keypair => 0,
            SignMode::Keyless => 1,
        });
        out.extend_from_slice(&self.manifest_root);
        out.extend_from_slice(&self.public_key);
        out.extend_from_slice(&self.signature);
        out
    }

    /// Decode a `.limsig` sidecar produced by [`Self::to_limsig`].
    ///
    /// # Errors
    ///
    /// Returns [`SignError::Crypto`] on length, magic, or version
    /// mismatch.
    pub fn from_limsig(bytes: &[u8]) -> Result<Self, SignError> {
        if bytes.len() != LIMSIG_LEN {
            return Err(SignError::Crypto(format!(
                "limsig: expected {LIMSIG_LEN} bytes, got {}",
                bytes.len()
            )));
        }
        if bytes[..4] != LIMSIG_MAGIC {
            return Err(SignError::Crypto("limsig: bad magic".into()));
        }
        if bytes[4] != 1 {
            return Err(SignError::Crypto(format!(
                "limsig: unsupported version {}",
                bytes[4]
            )));
        }
        let mut copy32 = |off: usize| -> [u8; 32] {
            let mut a = [0u8; 32];
            a.copy_from_slice(&bytes[off..off + 32]);
            a
        };
        Ok(Self {
            manifest_root: copy32(6),
            public_key: copy32(38),
            signature: {
                let mut a = [0u8; 64];
                a.copy_from_slice(&bytes[70..134]);
                a
            },
            mode: if bytes[5] == 0 {
                SignMode::Keypair
            } else {
                SignMode::Keyless
            },
        })
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

    #[test]
    fn pem_private_round_trip_matches_openssl_layout() {
        let seed: [u8; 32] = core::array::from_fn(|i| (i * 7 + 3) as u8);
        let pem = encode_private_pkcs8_pem(&seed);
        assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----\n"));
        assert!(pem.ends_with("-----END PRIVATE KEY-----\n"));
        let back = decode_private_pkcs8_pem(&pem).expect("decode");
        assert_eq!(back, seed);
        // Truncated / wrong label must fail.
        assert!(decode_private_pkcs8_pem(
            "-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----"
        )
        .is_err());
    }

    #[test]
    fn pem_public_round_trip() {
        let public: [u8; 32] = core::array::from_fn(|i| (i * 13 + 1) as u8);
        let pem = encode_public_spki_pem(&public);
        assert!(pem.starts_with("-----BEGIN PUBLIC KEY-----\n"));
        let back = decode_public_spki_pem(&pem).expect("decode");
        assert_eq!(back, public);
    }

    #[test]
    fn limsig_sidecar_round_trip_and_rejects_garbage() {
        let seed: [u8; 32] = core::array::from_fn(|i| i as u8);
        let kp = SigningKeyPair::from_bytes(seed);
        let root = [7u8; 32];
        let bundle = sign(&kp, &root).expect("sign");
        let bytes = bundle.to_limsig();
        assert_eq!(bytes.len(), LIMSIG_LEN);
        let back = SignatureBundle::from_limsig(&bytes).expect("decode");
        assert_eq!(back, bundle);
        assert!(SignatureBundle::from_limsig(&bytes[..LIMSIG_LEN - 1]).is_err());
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert!(SignatureBundle::from_limsig(&bad).is_err());
        verify(&back).expect("verify");
    }
}
