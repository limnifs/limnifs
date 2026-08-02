//! `IPFS` locator + CAR (Content Addressable aRchive) interop.
//!
//! Phase 3 v1: provides `IPFS` gateway fetching and a `CARv1` codec for
//! exporting/importing slab bytes plus their `BLAKE3`-multihash CIDs.
//!
//! ## Multihash bridging
//!
//! `DropId` is `BLAKE3(plaintext)`. The `IPFS` multihash registry assigns
//! code `0x1e` to `BLAKE3-256`, so every `DropId` maps 1:1 to a multihash
//! of the form `<0x1e><0x20><32 bytes>` (no transformation needed —
//! the digest is its own multihash body).
//!
//! ## CID encoding
//!
//! CIDs are `CIDv1` with codec `0x55` (raw): `<version=1><codec=0x55><multihash>`.
//! For `BLAKE3-256` drops, this is `<0x01><0x55><0x1e><0x20><32 bytes>`.
//!
//! ## CAR format
//!
//! `CARv1` layout (`IPLD` spec, `car.v1`):
//!
//! ```text
//! +----------------------------------+
//! | header uvarint-length + CBOR      |  roots: [CID], version: 1
//! +----------------------------------+
//! | block 1: uvarint-len + CID + data|
//! +----------------------------------+
//! | block 2: ...                     |
//! +----------------------------------+
//! ...
//! ```
//!
//! Each block's length prefix covers CID + data; CID has its own
//! self-delimiting length within.
//!
//! See task `08-ipfs-car.md`.

#![cfg(feature = "http")]
#![allow(clippy::doc_markdown)]

use crate::fetch::{Locator, LocatorError};
use crate::http_locator::HttpLocator;

/// Multihash code for BLAKE3-256.
pub const MULTIHASH_BLAKE3_256: u64 = 0x1e;

/// CID codec for raw bytes.
pub const CID_CODEC_RAW: u64 = 0x55;

/// CID version (always 1 in this module).
pub const CID_VERSION: u64 = 1;

/// Default IPFS HTTP gateway. Callers can override with
/// [`IpfsLocator::with_gateway`].
pub const DEFAULT_IPFS_GATEWAY: &str = "https://ipfs.io";

/// An IPFS gateway locator: fetches bytes by CID via an HTTP gateway.
///
/// v1 targets public gateways (default `https://ipfs.io`). The URI
/// scheme is `ipfs://<cid>/<path>`; the gateway URL becomes
/// `<gateway>/ipfs/<cid>/<path>`. Direct Kubo RPC (`/api/v0/dag/get`)
/// support is deferred to v2 (requires POST + multipart form data).
#[derive(Debug, Clone)]
pub struct IpfsLocator {
    gateway: String,
    inner: HttpLocator,
}

impl IpfsLocator {
    /// Create an IPFS locator pointing at the default gateway.
    #[must_use]
    pub fn new() -> Self {
        Self::with_gateway_and_agent(DEFAULT_IPFS_GATEWAY, HttpLocator::new())
    }

    /// Create an IPFS locator pointing at a custom gateway (e.g.
    /// `https://dweb.link`, a local Kubo node at `http://localhost:8080`).
    #[must_use]
    pub fn with_gateway(gateway: impl Into<String>) -> Self {
        Self::with_gateway_and_agent(gateway, HttpLocator::new())
    }

    /// Compose an IPFS locator with a pre-built [`HttpLocator`].
    #[must_use]
    pub fn with_gateway_and_agent(gateway: impl Into<String>, inner: HttpLocator) -> Self {
        let mut gateway = gateway.into();
        if gateway.ends_with('/') {
            gateway.pop();
        }
        Self { gateway, inner }
    }

    fn translate(&self, uri: &str) -> Result<String, LocatorError> {
        let rest = uri
            .strip_prefix("ipfs://")
            .ok_or_else(|| LocatorError::UnsupportedScheme {
                scheme: scheme_of(uri).to_owned(),
            })?;
        if rest.is_empty() {
            return Err(LocatorError::InvalidUri {
                reason: "ipfs URI has empty CID".into(),
            });
        }
        Ok(format!("{}/ipfs/{rest}", self.gateway))
    }
}

impl Default for IpfsLocator {
    fn default() -> Self {
        Self::new()
    }
}

impl Locator for IpfsLocator {
    fn fetch(&self, uri: &str) -> Result<Vec<u8>, LocatorError> {
        let http_uri = self.translate(uri)?;
        self.inner.fetch(&http_uri)
    }

    fn fetch_range(&self, uri: &str, offset: u64, length: u64) -> Result<Vec<u8>, LocatorError> {
        let http_uri = self.translate(uri)?;
        self.inner.fetch_range(&http_uri, offset, length)
    }

    fn scheme(&self) -> &'static str {
        "ipfs"
    }
}

fn scheme_of(uri: &str) -> &str {
    match uri.find(':') {
        Some(pos) => &uri[..pos],
        None => "",
    }
}

/// Encode a u64 as an unsigned LEB128 varint into `out`.
///
/// # Panics
///
/// Cannot panic: `(value & 0x7F) | 0x80` is always within `0x80..=0xFF`,
/// and `value` after the `while` loop is `< 0x80`. Both `try_from`
/// calls are statically satisfiable.
pub fn write_varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push(u8::try_from((value & 0x7F) | 0x80).expect("masked to low 7 bits"));
        value >>= 7;
    }
    out.push(u8::try_from(value).expect("high bits are zero"));
}

/// Decode an unsigned LEB128 varint from `bytes` starting at `pos`.
///
/// Returns `(value, bytes_consumed)`.
///
/// # Errors
///
/// - [`CarError::TruncatedVarint`] if the slice ends before the
///   varint terminator (high bit clear) is seen.
/// - [`CarError::VarintTooLong`] if more than 10 continuation bytes
///   appear (a u64 cannot need more).
pub fn read_varint(bytes: &[u8]) -> Result<(u64, usize), CarError> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if i >= 10 {
            return Err(CarError::VarintTooLong);
        }
        value |= u64::from(b & 0x7F)
            .checked_shl(shift)
            .ok_or(CarError::VarintOverflow)?;
        if b & 0x80 == 0 {
            return Ok((value, i + 1));
        }
        shift += 7;
    }
    Err(CarError::TruncatedVarint)
}

/// Encode a BLAKE3-256 multihash: `<0x1e><0x20><32 bytes>`.
#[must_use]
pub fn encode_blake3_multihash(digest: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(34);
    write_varint(MULTIHASH_BLAKE3_256, &mut out);
    write_varint(32, &mut out);
    out.extend_from_slice(digest);
    out
}

/// Decode a BLAKE3-256 multihash. Returns the 32-byte digest.
///
/// # Errors
///
/// - [`CarError::UnsupportedMultihash`] if the multihash code is not
///   BLAKE3-256.
/// - [`CarError::TruncatedMultihash`] if the bytes run out mid-decode.
pub fn decode_blake3_multihash(bytes: &[u8]) -> Result<[u8; 32], CarError> {
    let (code, consumed_code) = read_varint(bytes)?;
    if code != MULTIHASH_BLAKE3_256 {
        return Err(CarError::UnsupportedMultihash { code });
    }
    let (len, consumed_len) = read_varint(&bytes[consumed_code..])?;
    if len != 32 {
        return Err(CarError::UnsupportedMultihash { code: len });
    }
    let start = consumed_code + consumed_len;
    let end = start + 32;
    if bytes.len() < end {
        return Err(CarError::TruncatedMultihash);
    }
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&bytes[start..end]);
    Ok(digest)
}

/// Encode a `CIDv1` with the raw codec for a `BLAKE3-256` digest.
#[must_use]
pub fn encode_raw_cid(digest: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(36);
    write_varint(CID_VERSION, &mut out);
    write_varint(CID_CODEC_RAW, &mut out);
    out.extend_from_slice(&encode_blake3_multihash(digest));
    out
}

/// Decode a `CIDv1` raw `BLAKE3-256` from the given bytes.
///
/// Returns `(digest, bytes_consumed)`.
///
/// # Errors
///
/// - [`CarError::UnsupportedCidVersion`] if version != 1.
/// - [`CarError::UnsupportedCodec`] if codec != raw (0x55).
/// - Other errors bubble up from multihash decoding.
pub fn decode_raw_cid(bytes: &[u8]) -> Result<([u8; 32], usize), CarError> {
    let (version, consumed_v) = read_varint(bytes)?;
    if version != CID_VERSION {
        return Err(CarError::UnsupportedCidVersion { version });
    }
    let (codec, consumed_c) = read_varint(&bytes[consumed_v..])?;
    if codec != CID_CODEC_RAW {
        return Err(CarError::UnsupportedCodec { codec });
    }
    let mh_start = consumed_v + consumed_c;
    let digest = decode_blake3_multihash(&bytes[mh_start..])?;
    // The multihash body is fixed-size: <code><len><32 bytes>.
    // 2 varints (1 byte each for these small values) + 32 bytes.
    let consumed_total = mh_start + 2 + 32;
    Ok((digest, consumed_total))
}

/// A single CAR block: a `CID` + its raw data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CarBlock {
    pub digest: [u8; 32],
    pub data: Vec<u8>,
}

/// Encode a `CARv1` archive from a list of `(digest, data)` blocks.
///
/// The CAR header carries a single root `CID` (the first block's
/// digest). All blocks are encoded with codec `0x55` (raw).
///
/// # Panics
///
/// Panics if the encoded header or any block's length-prefix overflows
/// a `u64`. Headers are tiny (a few dozen bytes); blocks are bounded
/// by the caller-supplied slice. Neither overflow is reachable in
/// practice.
#[must_use]
pub fn encode_car_v1(blocks: &[CarBlock]) -> Vec<u8> {
    let mut out = Vec::new();

    // Header: CBOR map { "version": 1, "roots": [CID_of_first_block] }
    // For simplicity we hand-encode this canonical CBOR. The IPFS
    // spec requires a 2-element map with "version" and "roots", but
    // our reader is symmetric so we don't need a third-party CBOR
    // codec for round-trip.
    let root_cid = blocks
        .first()
        .map_or_else(Vec::new, |b| encode_raw_cid(&b.digest));
    let header = encode_car_header(&root_cid);
    write_varint(
        u64::try_from(header.len()).expect("header len fits u64"),
        &mut out,
    );
    out.extend_from_slice(&header);

    for block in blocks {
        let cid = encode_raw_cid(&block.digest);
        let block_inner_len = cid.len() + block.data.len();
        write_varint(
            u64::try_from(block_inner_len).expect("block len fits u64"),
            &mut out,
        );
        out.extend_from_slice(&cid);
        out.extend_from_slice(&block.data);
    }
    out
}

/// Decode a `CARv1` archive into a list of blocks.
///
/// # Errors
///
/// Returns [`CarError`] variants on malformed input.
pub fn decode_car_v1(bytes: &[u8]) -> Result<Vec<CarBlock>, CarError> {
    let (header_len, consumed_header_len) = read_varint(bytes)?;
    let header_len_us = usize::try_from(header_len).map_err(|_| CarError::LengthOverflow)?;
    let header_end = consumed_header_len
        .checked_add(header_len_us)
        .ok_or(CarError::LengthOverflow)?;
    if bytes.len() < header_end {
        return Err(CarError::TruncatedHeader);
    }
    // The header bytes are read but not parsed: our reader is
    // symmetric with our writer, and we trust the varint + bounds
    // checks above to validate the framing.
    let _ = consumed_header_len;
    let _ = header_end;

    let mut blocks = Vec::new();
    let mut pos = header_end;
    while pos < bytes.len() {
        let (block_inner_len, consumed_block_len) = read_varint(&bytes[pos..])?;
        let block_inner_len_us =
            usize::try_from(block_inner_len).map_err(|_| CarError::LengthOverflow)?;
        let block_start = pos
            .checked_add(consumed_block_len)
            .ok_or(CarError::LengthOverflow)?;
        let block_end = block_start
            .checked_add(block_inner_len_us)
            .ok_or(CarError::LengthOverflow)?;
        if bytes.len() < block_end {
            return Err(CarError::TruncatedBlock);
        }
        let block_bytes = &bytes[block_start..block_end];
        let (digest, consumed_cid) = decode_raw_cid(block_bytes)?;
        let data_start = consumed_cid;
        let data = block_bytes[data_start..].to_vec();
        blocks.push(CarBlock { digest, data });
        pos = block_end;
    }
    Ok(blocks)
}

/// Encode a minimal `CARv1` header as CBOR: `{"version": 1, "roots": [cid]}`.
fn encode_car_header(root_cid: &[u8]) -> Vec<u8> {
    // CBOR map of 2 entries.
    let mut out = Vec::new();
    out.push(0xa2); // map of 2 entries
                    // Key 1: "version" (text string of length 7)
    out.push(0x67); // text string of length 7
    out.extend_from_slice(b"version");
    // Value 1: unsigned int 1
    out.push(0x01);
    // Key 2: "roots" (text string of length 5)
    out.push(0x65); // text string of length 5
    out.extend_from_slice(b"roots");
    // Value 2: array of 1 byte-string (the CID).
    out.push(0x81); // array of 1 entry
    let cid_len_byte = u8::try_from(root_cid.len()).expect("CID len fits u8");
    out.push(0x40 | (0x1f & cid_len_byte)); // byte string of length cid_len
    out.extend_from_slice(root_cid);
    out
}

/// Errors from CAR/CID/multihash codec operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CarError {
    TruncatedVarint,
    VarintTooLong,
    VarintOverflow,
    UnsupportedMultihash { code: u64 },
    UnsupportedCidVersion { version: u64 },
    UnsupportedCodec { codec: u64 },
    TruncatedMultihash,
    TruncatedHeader,
    TruncatedBlock,
    LengthOverflow,
}

impl std::fmt::Display for CarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TruncatedVarint => write!(f, "CAR: truncated varint"),
            Self::VarintTooLong => write!(f, "CAR: varint exceeds 10 bytes"),
            Self::VarintOverflow => write!(f, "CAR: varint overflows u64"),
            Self::UnsupportedMultihash { code } => {
                write!(f, "CAR: unsupported multihash code {code:#x}")
            }
            Self::UnsupportedCidVersion { version } => {
                write!(f, "CAR: unsupported CID version {version}")
            }
            Self::UnsupportedCodec { codec } => write!(f, "CAR: unsupported CID codec {codec:#x}"),
            Self::TruncatedMultihash => write!(f, "CAR: truncated multihash"),
            Self::TruncatedHeader => write!(f, "CAR: truncated header"),
            Self::TruncatedBlock => write!(f, "CAR: truncated block"),
            Self::LengthOverflow => write!(f, "CAR: length overflow"),
        }
    }
}

impl std::error::Error for CarError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_round_trips_small() {
        for value in [0u64, 1, 42, 127, 128, 255, 256, 16383, 16384, 65535] {
            let mut buf = Vec::new();
            write_varint(value, &mut buf);
            let (decoded, consumed) = read_varint(&buf).expect("decodes");
            assert_eq!(decoded, value, "value={value}");
            assert_eq!(consumed, buf.len(), "value={value}");
        }
    }

    #[test]
    fn varint_round_trips_max() {
        let mut buf = Vec::new();
        write_varint(u64::MAX, &mut buf);
        let (decoded, _) = read_varint(&buf).expect("decodes");
        assert_eq!(decoded, u64::MAX);
    }

    #[test]
    fn varint_rejects_truncated() {
        // Single byte with continuation set, no terminator.
        let err = read_varint(&[0x80]).unwrap_err();
        assert_eq!(err, CarError::TruncatedVarint);
    }

    #[test]
    fn multihash_round_trips() {
        let digest = [0xABu8; 32];
        let encoded = encode_blake3_multihash(&digest);
        assert_eq!(encoded.len(), 34);
        let decoded = decode_blake3_multihash(&encoded).expect("decodes");
        assert_eq!(decoded, digest);
    }

    #[test]
    fn multihash_rejects_wrong_code() {
        // SHA2-256 multihash code is 0x12.
        let mut bad = Vec::new();
        write_varint(0x12, &mut bad);
        write_varint(32, &mut bad);
        bad.extend_from_slice(&[0u8; 32]);
        match decode_blake3_multihash(&bad) {
            Err(CarError::UnsupportedMultihash { code }) => assert_eq!(code, 0x12),
            other => panic!("expected UnsupportedMultihash, got {other:?}"),
        }
    }

    #[test]
    fn cid_round_trips() {
        let digest = [0x42u8; 32];
        let cid = encode_raw_cid(&digest);
        // version(1 byte) + codec(1 byte) + mh_code(1) + mh_len(1) + 32 = 36 bytes
        assert_eq!(cid.len(), 36);
        let (decoded_digest, consumed) = decode_raw_cid(&cid).expect("decodes");
        assert_eq!(decoded_digest, digest);
        assert_eq!(consumed, cid.len());
    }

    #[test]
    fn cid_rejects_wrong_version() {
        // CIDv0 would start with varint 0 (since 0 is its own varint).
        let mut bad = vec![0u8];
        bad.extend_from_slice(&encode_raw_cid(&[1u8; 32])[1..]);
        match decode_raw_cid(&bad) {
            Err(CarError::UnsupportedCidVersion { version }) => assert_eq!(version, 0),
            other => panic!("expected UnsupportedCidVersion, got {other:?}"),
        }
    }

    #[test]
    fn cid_rejects_wrong_codec() {
        let mut bad = Vec::new();
        write_varint(1, &mut bad);
        write_varint(0x70, &mut bad); // dag-pb
        bad.extend_from_slice(&encode_blake3_multihash(&[2u8; 32]));
        match decode_raw_cid(&bad) {
            Err(CarError::UnsupportedCodec { codec }) => assert_eq!(codec, 0x70),
            other => panic!("expected UnsupportedCodec, got {other:?}"),
        }
    }

    #[test]
    fn car_round_trip_empty() {
        // Empty CAR — no blocks, but a header with an empty roots list.
        // Our v1 encoder requires at least one block for the root CID.
        // Encode with a single empty block to verify the round-trip.
        let blocks = vec![CarBlock {
            digest: [0u8; 32],
            data: Vec::new(),
        }];
        let bytes = encode_car_v1(&blocks);
        let decoded = decode_car_v1(&bytes).expect("decodes");
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].digest, [0u8; 32]);
        assert!(decoded[0].data.is_empty());
    }

    #[test]
    fn car_round_trip_multiple_blocks() {
        let blocks = vec![
            CarBlock {
                digest: [0xAAu8; 32],
                data: b"first block".to_vec(),
            },
            CarBlock {
                digest: [0xBBu8; 32],
                data: b"second block".to_vec(),
            },
            CarBlock {
                digest: [0xCCu8; 32],
                data: vec![0u8; 256],
            },
        ];
        let bytes = encode_car_v1(&blocks);
        let decoded = decode_car_v1(&bytes).expect("decodes");
        assert_eq!(decoded, blocks);
    }

    #[test]
    fn car_preserves_drop_ids() {
        // The DropIds (BLAKE3 multihashes) must round-trip unchanged.
        // This is the identity rule for CAR interop.
        let blocks: Vec<CarBlock> = (0..5)
            .map(|i| CarBlock {
                digest: [u8::try_from(i).unwrap(); 32],
                data: vec![u8::try_from(i).unwrap_or(0); 64],
            })
            .collect();
        let bytes = encode_car_v1(&blocks);
        let decoded = decode_car_v1(&bytes).expect("decodes");
        for (orig, dec) in blocks.iter().zip(decoded.iter()) {
            assert_eq!(orig.digest, dec.digest);
            assert_eq!(orig.data, dec.data);
        }
    }

    #[test]
    fn ipfs_locator_translates_uri() {
        let loc = IpfsLocator::with_gateway("https://ipfs.io");
        let translated = loc
            .translate("ipfs://bafyabc123/path/to/object")
            .expect("translates");
        assert_eq!(translated, "https://ipfs.io/ipfs/bafyabc123/path/to/object");
    }

    #[test]
    fn ipfs_locator_strips_trailing_slash() {
        let loc = IpfsLocator::with_gateway("https://ipfs.io/");
        let translated = loc.translate("ipfs://cid123").expect("translates");
        assert_eq!(translated, "https://ipfs.io/ipfs/cid123");
    }

    #[test]
    fn ipfs_locator_rejects_empty_cid() {
        let loc = IpfsLocator::new();
        match loc.translate("ipfs://") {
            Err(LocatorError::InvalidUri { .. }) => {}
            other => panic!("expected InvalidUri, got {other:?}"),
        }
    }

    #[test]
    fn ipfs_locator_rejects_non_ipfs_scheme() {
        let loc = IpfsLocator::new();
        match loc.translate("https://example.com/x") {
            Err(LocatorError::UnsupportedScheme { scheme }) => assert_eq!(scheme, "https"),
            other => panic!("expected UnsupportedScheme, got {other:?}"),
        }
    }

    #[test]
    fn ipfs_locator_reports_scheme() {
        let loc = IpfsLocator::new();
        assert_eq!(loc.scheme(), "ipfs");
    }

    #[test]
    fn ipfs_locator_round_trip_via_local_gateway() {
        // Spin up a minimal HTTP server that pretends to be an IPFS
        // gateway and verify the locator fetches the bytes.
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let payload = b"ipfs-block-payload".to_vec();
        let payload_for_assert = payload.clone();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            // Drain request headers.
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                if line.trim().is_empty() {
                    break;
                }
            }
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                payload.len(),
            );
            sock.write_all(header.as_bytes()).unwrap();
            sock.write_all(&payload).unwrap();
        });

        let gateway = format!("http://{addr}");
        let locator = IpfsLocator::with_gateway(gateway);
        let data = locator.fetch("ipfs://bafyfakecid/slab.bin").expect("fetch");
        handle.join().unwrap();
        assert_eq!(data, payload_for_assert);
    }
}
