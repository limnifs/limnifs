//! Slab format v2 conformance vectors (TODO.sota-fs/05).
//!
//! Builds slabs directly on the wire (the single format) — a SEEKABLE drop (window
//! bytes are a `LMSK` container) and a plain drop sharing the slab —
//! and verifies the reader contract: full decode identity, ranged
//! decode identity, flag queries, and fail-closed corruption
//! handling. Complements the end-to-end writer→reader round-trips in
//! `limnifs-write/tests/seekable_drop_round_trip.rs`.

use limnifs_core::codec::{self, CodecTunables};
use limnifs_core::drop_record::NO_DICT;
use limnifs_core::seekable::{self, DROP_FLAG_SEEKABLE};
use limnifs_core::slab_store::SlabStore;

fn xorshift(bytes: usize, seed: u64) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(bytes);
    while out.len() < bytes {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(bytes);
    out
}

struct SlabFixture {
    slab: Vec<u8>,
    seekable_id: [u8; 32],
    seekable_pt: Vec<u8>,
    plain_id: [u8; 32],
    plain_pt: Vec<u8>,
    container_len: usize,
}

/// One v2 slab carrying a seekable drop and a plain drop.
fn build_slab() -> SlabFixture {
    let seekable_pt = xorshift(600 * 1024, 0xA1);
    let plain_pt = xorshift(64 * 1024, 0xB2);

    let seekable_id = limnifs_core::hash_section(&seekable_pt);
    let plain_id = limnifs_core::hash_section(&plain_pt);

    let container =
        seekable::encode_seekable(codec::CODEC_ZSTD, &seekable_pt, &CodecTunables::default())
            .expect("container encodes");
    let plain_comp = codec::compress(codec::CODEC_LZ4, &plain_pt).expect("lz4");

    let mut slab = Vec::new();
    slab.extend_from_slice(b"LIM1");
    slab.extend_from_slice(&1u16.to_le_bytes());
    slab.extend_from_slice(&[0u8; 8]); // ordinal 0
    slab.extend_from_slice(&[0u8; 32]); // content hash placeholder
    let total: u64 = (56 + 2 * 50 + container.len() + plain_comp.len()) as u64;
    slab.extend_from_slice(&total.to_le_bytes());
    slab.push(0x00);
    slab.push(0x00);
    // record 1: seekable
    slab.extend_from_slice(&seekable_id);
    slab.extend_from_slice(&(seekable_pt.len() as u32).to_le_bytes());
    slab.extend_from_slice(&[codec::CODEC_ZSTD, 0x00, 0x00]);
    slab.push(0x00);
    slab.extend_from_slice(&0u32.to_le_bytes());
    slab.extend_from_slice(&(container.len() as u32).to_le_bytes());
    slab.push(NO_DICT);
    slab.push(DROP_FLAG_SEEKABLE);
    // record 2: plain
    slab.extend_from_slice(&plain_id);
    slab.extend_from_slice(&(plain_pt.len() as u32).to_le_bytes());
    slab.extend_from_slice(&[codec::CODEC_LZ4, 0x00, 0x00]);
    slab.push(0x00);
    slab.extend_from_slice(&(container.len() as u32).to_le_bytes());
    slab.extend_from_slice(&(plain_comp.len() as u32).to_le_bytes());
    slab.push(NO_DICT);
    slab.push(0x00);
    // window
    slab.extend_from_slice(&container);
    slab.extend_from_slice(&plain_comp);

    SlabFixture {
        slab,
        seekable_id,
        seekable_pt,
        plain_id,
        plain_pt,
        container_len: container.len(),
    }
}

#[test]
fn slab_full_and_ranged_decode_identity() {
    let f = build_slab();
    let store = SlabStore::from_bytes(vec![f.slab]).expect("slab parses");
    assert_eq!(store.drop_count(), 2);

    assert_eq!(store.drop_is_seekable(&f.seekable_id), Some(true));
    assert_eq!(store.drop_is_seekable(&f.plain_id), Some(false));

    let got = store
        .plaintext_for(&f.seekable_id)
        .expect("drop")
        .expect("decode");
    assert_eq!(got, f.seekable_pt);
    let got = store
        .plaintext_for(&f.plain_id)
        .expect("drop")
        .expect("decode");
    assert_eq!(got, f.plain_pt);

    for (off, len) in [(0u64, 4096usize), (255 * 1024, 8192), (599 * 1024, 1024)] {
        let r = store
            .plaintext_range(&f.seekable_id, off, len)
            .expect("drop")
            .expect("range");
        assert_eq!(r, f.seekable_pt[off as usize..off as usize + len]);
    }
}

#[test]
fn corrupt_container_fails_closed() {
    let mut f = build_slab();
    // Flip a byte in the container's fixed footer tail (magic /
    // version / frame_count live in the last bytes of the seekable
    // drop's window).
    let window_start = 56 + 2 * 50;
    let idx = window_start + f.container_len - 1;
    f.slab[idx] ^= 0xFF;
    let store = SlabStore::from_bytes(vec![f.slab]).expect("slab still parses structurally");
    let err = store
        .plaintext_for(&f.seekable_id)
        .expect("drop present")
        .expect_err("corrupt footer must fail decode");
    assert!(
        err.to_string().contains("seekable"),
        "error should name the container: {err}"
    );
}
