//! Semantic types for the `LimniFS` wire format.
//!
//! Source of truth: `limnifs/spec` §2.2 (Terminology / semantic types).
//! Each type is emitted as a distinct newtype (not a bare alias) so the
//! per-field semantic constraints from the spec (§1.1 multihash display,
//! §1.4 determinism, exact widths) are enforced at module boundaries.
//!
//! All widths are exact. Storage MUST NOT widen; width changes are spec
//! amendments, not implementation choices.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use core::fmt;

const BLAKE3_LEN: usize = 32;
const ORDINAL_LEN: usize = 8;
const SLAB_ID_LEN: usize = ORDINAL_LEN + BLAKE3_LEN;

pub const MANIFEST_MAGIC: [u8; 4] = *b"LMFS";
pub const SLAB_MAGIC: [u8; 4] = *b"LIM1";

pub const MANIFEST_HEADER_LEN: usize = 16;

/// RFC 4648 base32 lowercase alphabet, no padding.
const BASE32_LOWER: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

fn encode_base32_lower_no_pad(input: &[u8]) -> String {
    let capacity = (input.len() * 8).div_ceil(5);
    let mut out = String::with_capacity(capacity);
    let mut buffer: u64 = 0;
    let mut bits: u32 = 0;
    for &byte in input {
        buffer = (buffer << 8) | u64::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let shift = buffer >> bits;
            let idx = (shift & 0x1F) as usize;
            out.push(BASE32_LOWER[idx] as char);
        }
    }
    if bits > 0 {
        let shift = 5 - bits;
        let idx = ((buffer << shift) & 0x1F) as usize;
        out.push(BASE32_LOWER[idx] as char);
    }
    out
}

fn decode_base32_lower_no_pad(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 5 / 8 + 1);
    let mut buffer: u64 = 0;
    let mut bits: u32 = 0;
    for ch in input.chars() {
        let val = match ch {
            'a'..='z' => u32::from(ch) - u32::from('a'),
            '2'..='7' => u32::from(ch) - u32::from('2') + 26,
            _ => return None,
        };
        buffer = (buffer << 5) | u64::from(val);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            let shift = buffer >> bits;
            out.push((shift & 0xFF) as u8);
        }
    }
    if bits >= 5 || (buffer & ((1 << bits) - 1) != 0) {
        return None;
    }
    Some(out)
}

/// 32-byte BLAKE3-derived drop identity.
///
/// Per §1.1: `DropId = BLAKE3(plaintext)`. The display form is
/// `b3:<base32-lower-no-pad>` (multihash-compatible, multibase-decodable).
/// Identity is independent of every representation choice (§1.3).
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DropId([u8; BLAKE3_LEN]);

impl DropId {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; BLAKE3_LEN]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; BLAKE3_LEN] {
        &self.0
    }

    /// Parse the multihash text form `b3:<base32-lower-no-pad>`.
    ///
    /// Returns `None` if the prefix, alphabet, or output length is wrong.
    #[must_use]
    pub fn parse_text(s: &str) -> Option<Self> {
        let rest = s.strip_prefix("b3:")?;
        let bytes = decode_base32_lower_no_pad(rest)?;
        if bytes.len() != BLAKE3_LEN {
            return None;
        }
        let mut arr = [0u8; BLAKE3_LEN];
        arr.copy_from_slice(&bytes);
        Some(Self(arr))
    }

    #[must_use]
    pub fn to_text(self) -> String {
        let mut s = String::with_capacity(3 + (BLAKE3_LEN * 8).div_ceil(5));
        s.push_str("b3:");
        s.push_str(&encode_base32_lower_no_pad(&self.0));
        s
    }
}

impl fmt::Display for DropId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}

impl fmt::Debug for DropId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DropId({self})")
    }
}

/// Per-image slab identifier: 8-byte ordinal + 32-byte content hash.
///
/// Per §2.2: the ordinal ensures distinct slabs that hash to the same
/// value within one image remain distinguishable. Total width is 40 bytes.
#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct SlabId {
    pub ordinal: u64,
    pub hash: [u8; BLAKE3_LEN],
}

impl SlabId {
    #[must_use]
    pub const fn new(ordinal: u64, hash: [u8; BLAKE3_LEN]) -> Self {
        Self { ordinal, hash }
    }

    #[must_use]
    pub fn from_bytes(bytes: &[u8; SLAB_ID_LEN]) -> Self {
        let mut ordinal_bytes = [0u8; ORDINAL_LEN];
        ordinal_bytes.copy_from_slice(&bytes[..ORDINAL_LEN]);
        let mut hash = [0u8; BLAKE3_LEN];
        hash.copy_from_slice(&bytes[ORDINAL_LEN..]);
        Self {
            ordinal: u64::from_le_bytes(ordinal_bytes),
            hash,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; SLAB_ID_LEN] {
        let mut out = [0u8; SLAB_ID_LEN];
        out[..ORDINAL_LEN].copy_from_slice(&self.ordinal.to_le_bytes());
        out[ORDINAL_LEN..].copy_from_slice(&self.hash);
        out
    }
}

impl fmt::Debug for SlabId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlabId")
            .field("ordinal", &self.ordinal)
            .field("hash", &encode_base32_lower_no_pad(&self.hash))
            .finish()
    }
}

/// 32-byte image identity: BLAKE3 of the manifest's Merkle hash list.
///
/// Per §1.2: `ManifestRoot` is the only handle by which an image is
/// identified at rest and in transit. Display form matches `DropId`
/// (§1.1), but the type is distinct so module boundaries cannot confuse
/// image identity with drop identity.
#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct ManifestRoot([u8; BLAKE3_LEN]);

impl ManifestRoot {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; BLAKE3_LEN]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; BLAKE3_LEN] {
        &self.0
    }

    #[must_use]
    pub fn to_text(self) -> String {
        let mut s = String::with_capacity(3 + (BLAKE3_LEN * 8).div_ceil(5));
        s.push_str("b3:");
        s.push_str(&encode_base32_lower_no_pad(&self.0));
        s
    }
}

impl fmt::Display for ManifestRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}

impl fmt::Debug for ManifestRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ManifestRoot({self})")
    }
}

/// Per-slab tier, limnologic vocabulary (§2.1).
///
/// Wire encoding is a single byte per §2.2. Unknown bytes MUST be
/// rejected by readers with `Unsupported`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum Tier {
    Epilimnion = 0x00,
    Metalimnion = 0x01,
    Hypolimnion = 0x02,
}

impl Tier {
    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(Self::Epilimnion),
            0x01 => Some(Self::Metalimnion),
            0x02 => Some(Self::Hypolimnion),
            _ => None,
        }
    }

    #[must_use]
    pub const fn to_byte(self) -> u8 {
        self as u8
    }
}

/// Codec, AEAD, and EC identifiers selecting how a representation was
/// encoded (§1.3). Per §2.2: codec id (1), aead id (1, `0x00` = none),
/// ec id (1, `0x00` = none). Total width is 3 bytes.
///
/// Registry IDs are normative; see `limnifs/spec` §10 (AEAD), §11 (codec),
/// §14 (feature-flag/EC variants).
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Representation {
    pub codec: u8,
    pub aead: u8,
    pub ec: u8,
}

impl Representation {
    pub const STORE_PLAINTEXT: Self = Self {
        codec: 0x00,
        aead: 0x00,
        ec: 0x00,
    };

    #[must_use]
    pub const fn new(codec: u8, aead: u8, ec: u8) -> Self {
        Self { codec, aead, ec }
    }

    #[must_use]
    pub fn is_plaintext(self) -> bool {
        self.aead == 0x00
    }

    #[must_use]
    pub fn has_no_ec(self) -> bool {
        self.ec == 0x00
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; 3] {
        [self.codec, self.aead, self.ec]
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 3]) -> Self {
        Self {
            codec: bytes[0],
            aead: bytes[1],
            ec: bytes[2],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_roundtrip_zero_bytes() {
        let input = [0u8; 32];
        let encoded = encode_base32_lower_no_pad(&input);
        assert_eq!(encoded.len(), (32u32 * 8).div_ceil(5) as usize);
        let decoded = decode_base32_lower_no_pad(&encoded).expect("roundtrip");
        assert_eq!(decoded, input.to_vec());
    }

    #[test]
    fn base32_roundtrip_mixed_bytes() {
        let input: Vec<u8> = (0..32u8).collect();
        let encoded = encode_base32_lower_no_pad(&input);
        let decoded = decode_base32_lower_no_pad(&encoded).expect("roundtrip");
        assert_eq!(decoded, input);
    }

    #[test]
    fn base32_rejects_invalid_chars() {
        assert!(decode_base32_lower_no_pad("0").is_none());
        assert!(decode_base32_lower_no_pad("1").is_none());
        assert!(decode_base32_lower_no_pad("8").is_none());
        assert!(decode_base32_lower_no_pad("!").is_none());
    }

    #[test]
    fn base32_known_vector() {
        // RFC 4648 §10 test vectors (lowercase, no padding):
        assert_eq!(encode_base32_lower_no_pad(b"f"), "my");
        assert_eq!(encode_base32_lower_no_pad(b"fo"), "mzxq");
        assert_eq!(encode_base32_lower_no_pad(b"foo"), "mzxw6");
        assert_eq!(encode_base32_lower_no_pad(b"foob"), "mzxw6yq");
        assert_eq!(encode_base32_lower_no_pad(b"fooba"), "mzxw6ytb");
        assert_eq!(encode_base32_lower_no_pad(b"foobar"), "mzxw6ytboi");
    }

    #[test]
    fn drop_id_text_roundtrip() {
        let bytes = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
            0x1c, 0x1d, 0x1e, 0x1f,
        ];
        let id = DropId::from_bytes(bytes);
        let text = id.to_text();
        assert!(text.starts_with("b3:"));
        let parsed = DropId::parse_text(&text).expect("roundtrip");
        assert_eq!(parsed, id);
    }

    #[test]
    fn drop_id_parse_rejects_wrong_prefix() {
        assert!(DropId::parse_text("zb3:aaaa").is_none());
        assert!(DropId::parse_text("aaaa").is_none());
    }

    #[test]
    fn drop_id_parse_rejects_wrong_length() {
        assert!(DropId::parse_text("b3:my").is_none());
    }

    #[test]
    fn slab_id_roundtrip() {
        let id = SlabId::new(0x0123_4567_89ab_cdef, [0xaa; 32]);
        let bytes = id.to_bytes();
        assert_eq!(bytes.len(), SLAB_ID_LEN);
        let back = SlabId::from_bytes(&bytes);
        assert_eq!(back, id);
        assert_eq!(back.ordinal, 0x0123_4567_89ab_cdef);
        assert_eq!(back.hash, [0xaa; 32]);
    }

    #[test]
    fn slab_id_ordinal_is_little_endian() {
        let id = SlabId::new(1, [0; 32]);
        let bytes = id.to_bytes();
        assert_eq!(bytes[0], 0x01);
        for byte in &bytes[1..8] {
            assert_eq!(*byte, 0);
        }
    }

    #[test]
    fn manifest_root_display_matches_drop_id_format() {
        let root = ManifestRoot::from_bytes([0x42; 32]);
        let drop_id = DropId::from_bytes([0x42; 32]);
        assert_eq!(root.to_text(), drop_id.to_text());
    }

    #[test]
    fn tier_roundtrip() {
        assert_eq!(Tier::from_byte(0x00), Some(Tier::Epilimnion));
        assert_eq!(Tier::from_byte(0x01), Some(Tier::Metalimnion));
        assert_eq!(Tier::from_byte(0x02), Some(Tier::Hypolimnion));
        assert_eq!(Tier::from_byte(0x03), None);
        assert_eq!(Tier::Epilimnion.to_byte(), 0x00);
        assert_eq!(Tier::Metalimnion.to_byte(), 0x01);
        assert_eq!(Tier::Hypolimnion.to_byte(), 0x02);
    }

    #[test]
    fn representation_store_plaintext_constant() {
        let r = Representation::STORE_PLAINTEXT;
        assert!(r.is_plaintext());
        assert!(r.has_no_ec());
        assert_eq!(r.codec, 0x00);
        assert_eq!(r.aead, 0x00);
        assert_eq!(r.ec, 0x00);
    }

    #[test]
    fn representation_byte_roundtrip() {
        let r = Representation::new(0x01, 0x02, 0x03);
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x01, 0x02, 0x03]);
        let back = Representation::from_bytes(bytes);
        assert_eq!(back, r);
    }

    #[test]
    fn representation_predicate_methods() {
        let plaintext = Representation::new(0x01, 0x00, 0x05);
        assert!(plaintext.is_plaintext());
        assert!(!plaintext.has_no_ec());

        let sealed = Representation::new(0x01, 0x02, 0x00);
        assert!(!sealed.is_plaintext());
        assert!(sealed.has_no_ec());
    }

    #[test]
    fn magic_constants_match_spec() {
        assert_eq!(&MANIFEST_MAGIC, b"LMFS");
        assert_eq!(&SLAB_MAGIC, b"LIM1");
        assert_eq!(MANIFEST_HEADER_LEN, 16);
    }
}
