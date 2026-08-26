//! Slab format v2 end-to-end: seekable containers on the write side,
//! bounded windowed decode on the read side (TODO.sota-fs/05).

use limnifs_core::read::{ImageReader, ReadConfig};
use limnifs_core::seekable;
use limnifs_core::slab_store::SlabStore;
use limnifs_write::{write_directory_with_config, WriteConfig};

/// Low-entropy variant: one fresh byte per 8 (LZ4-compressible), so
/// fixtures take the container path instead of the STORE fallback.
fn xorshift_compressible(bytes: usize) -> Vec<u8> {
    let mut state = 0x0123_4567_89AB_CDEFu64;
    let mut out = Vec::with_capacity(bytes);
    while out.len() < bytes {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(&[state.to_le_bytes()[7]; 8]);
    }
    out.truncate(bytes);
    out
}

struct Image {
    path: std::path::PathBuf,
}

fn pack(src: &std::path::Path, skip_chunking: bool) -> Image {
    let mut config = WriteConfig::default_v0_1();
    config.skip_chunking = skip_chunking;
    // LZ4 keeps the pack fast on incompressible input (default
    // brotli-HQ spends minutes zopfli-parsing random bytes).
    config.defaults.text_codec = "lz4".into();
    config.defaults.binary_codec = "lz4".into();
    let art = write_directory_with_config(src, &config).expect("write");
    let dir = std::env::temp_dir().join(format!(
        "limnifs-seekable-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos())
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("image.lim"), &art.bytes).expect("manifest");
    for slab in &art.slabs {
        let name =
            limnifs_core::locator::local_sidecar_name(&slab.locator).expect("flat slab name");
        std::fs::write(dir.join(name), &slab.bytes).expect("slab");
    }
    if let Some(side) = &art.metadata_sidecar {
        let name =
            limnifs_core::locator::local_sidecar_name(&side.locator).expect("flat sidecar name");
        std::fs::write(dir.join(name), &side.bytes).expect("metadata");
    }
    Image {
        path: dir.join("image.lim"),
    }
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "limnifs-seekable-src-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos())
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[test]
fn v2_image_round_trips_and_cold_windows_decode_one_frame() {
    // 19.5 MiB — the tebako #192 scenario size. skip_chunking makes
    // the whole file ONE drop, which now emits as a seekable
    // container. The small file stays inline, so the image mixes
    // seekable + inline content.
    let big = xorshift_compressible(19 * 1024 * 1024 + 512 * 1024); // 19.5 MiB
    let big_len = big.len();
    let src = scratch("v2");
    std::fs::write(src.join("big.bin"), &big).expect("write big");
    std::fs::write(src.join("small.txt"), b"tiny inline payload").expect("write small");

    let img = pack(&src, true);
    let reader = ImageReader::open(&img.path, ReadConfig::default()).expect("open");

    // Sequential extract is byte-exact.
    let mut file = reader.file("/big.bin").expect("file");
    assert_eq!(file.size(), big_len as u64);
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut out).expect("read");
    assert_eq!(out, big);

    // Inline cohabitant still reads.
    let mut small = reader.file("/small.txt").expect("small");
    let mut small_out = Vec::new();
    std::io::Read::read_to_end(&mut small, &mut small_out).expect("read small");
    assert_eq!(small_out, b"tiny inline payload");

    // Cold random windows: each 8 KiB window on the 19.5 MiB seekable
    // drop decompresses at most ONE 256 KiB frame. The sequential
    // read above warmed the full-drop cache, so assert through a
    // fresh reader with a one-entry cache.
    let cold = ImageReader::open(
        &img.path,
        ReadConfig {
            cache_entries: 1,
            cache_bytes: 1, // every full drop bypasses -> true cold path
            parallel_decode: false,
            frame_cache_bytes: 1, // ... and frames bypass: cold windows
        },
    )
    .expect("open cold");
    let file = cold.file("/big.bin").expect("file");
    let mut window = vec![0u8; 8192];
    let mut state = 0xDEAD_BEEF_CAFE_F00Du64;
    let mut total_frames = 0u64;
    for _ in 0..16 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let off = (state % (big_len as u64 - 8192)) as usize;
        let before = seekable::frames_decoded();
        let n = file.read_at(off as u64, &mut window).expect("read_at");
        let touched = seekable::frames_decoded() - before;
        total_frames += touched;
        assert_eq!(&window[..n], &big[off..off + n], "off={off}");
        assert!(
            touched <= 1,
            "cold 8 KiB window at {off} decoded {touched} frames, expected <= 1"
        );
    }
    // The windows actually exercised the container path (a STORE
    // fallback drop would decode zero frames and pass vacuously).
    assert!(total_frames >= 1, "fixture must produce a seekable drop");

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(img.path.parent().expect("img parent"));
}

#[test]
fn pre_frozen_layout_slabs_fail_closed() {
    // Single-format call (TODO.sota-fs/08): the pre-seekable layout
    // (49-byte records under the same format_version 1) is NOT a
    // format we read. It must fail closed, not misparse.
    let plaintext = vec![0xCDu8; 8192];
    let drop_id = limnifs_core::hash_section(&plaintext);
    let compressed =
        limnifs_core::codec::compress(limnifs_core::codec::CODEC_LZ4, &plaintext).expect("lz4");

    let mut slab = Vec::new();
    slab.extend_from_slice(b"LIM1");
    slab.extend_from_slice(&1u16.to_le_bytes());
    slab.extend_from_slice(&[0u8; 8]);
    slab.extend_from_slice(&drop_id);
    let total: u64 = 56 + 49 + compressed.len() as u64; // 49-byte record: pre-frozen layout
    slab.extend_from_slice(&total.to_le_bytes());
    slab.push(0x00);
    slab.push(0x00);
    slab.extend_from_slice(&drop_id);
    slab.extend_from_slice(&(plaintext.len() as u32).to_le_bytes());
    slab.push(limnifs_core::codec::CODEC_LZ4);
    slab.push(0x00);
    slab.push(0x00);
    slab.push(0x00);
    slab.extend_from_slice(&0u32.to_le_bytes());
    slab.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    slab.push(limnifs_core::drop_record::NO_DICT);
    slab.extend_from_slice(&compressed);

    let err = SlabStore::from_bytes(vec![slab]).expect_err("old layout must be rejected");
    assert!(
        err.to_string().to_lowercase().contains("corrupt"),
        "expected a corruption error, got: {err}"
    );
}

#[test]
fn frame_cache_makes_repeat_windows_free() {
    // F2 (TODO.sota-fs/09): after the first window touching a frame,
    // repeat windows inside the same frame must decode ZERO new
    // frames — they resolve from the frame cache.
    let big = xorshift_compressible(2 * 1024 * 1024);
    let src = scratch("framecache");
    std::fs::write(src.join("big.bin"), &big).expect("write");
    let img = pack(&src, true);
    let reader = ImageReader::open(&img.path, ReadConfig::default()).expect("open");
    let file = reader.file("/big.bin").expect("file");
    let mut window = vec![0u8; 8192];
    let frame_base = 300 * 1024u64; // somewhere inside frame 1
    let before = seekable::frames_decoded();
    let n = file.read_at(frame_base, &mut window).expect("first read");
    assert_eq!(
        &window[..n],
        &big[frame_base as usize..frame_base as usize + n]
    );
    let first_cost = seekable::frames_decoded() - before;
    assert!(first_cost >= 1, "first window must decode its frame");

    // Same frame, different offsets — no new decodes.
    let before = seekable::frames_decoded();
    for delta in [1u64, 4096, 100_000] {
        let n = file
            .read_at(frame_base + delta, &mut window)
            .expect("repeat read");
        assert_eq!(
            &window[..n],
            &big[(frame_base + delta) as usize..(frame_base + delta) as usize + n]
        );
    }
    let repeat_cost = seekable::frames_decoded() - before;
    assert_eq!(
        repeat_cost, 0,
        "repeat windows in a cached frame must not re-decode"
    );

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(img.path.parent().expect("img parent"));
}

#[test]
fn seekable_drops_false_emits_monolithic() {
    // F5: the knob produces a monolithic drop with flags=0 that
    // still round-trips byte-exact.
    let big = xorshift_compressible(2 * 1024 * 1024);
    let src = scratch("monolithic");
    std::fs::write(src.join("big.bin"), &big).expect("write");
    let mut config = WriteConfig::default_v0_1();
    config.skip_chunking = true;
    config.defaults.text_codec = "lz4".into();
    config.defaults.binary_codec = "lz4".into();
    config.defaults.seekable_drops = false;
    let art = write_directory_with_config(&src, &config).expect("write");
    assert_eq!(art.drop_count, 1);

    let img = pack(&src, false);
    // pack() builds its own config; rebuild the monolithic image
    // manually since pack() flips codecs but keeps seekable_drops.
    let dir = img.path.parent().expect("img parent");
    std::fs::write(dir.join("image.lim"), &art.bytes).expect("manifest");
    for slab in &art.slabs {
        let name =
            limnifs_core::locator::local_sidecar_name(&slab.locator).expect("flat slab locator");
        std::fs::write(dir.join(name), &slab.bytes).expect("slab");
    }
    // The drop is monolithic: no SEEKABLE flag on its record.
    let slab_path = {
        let mut p = None;
        for entry in std::fs::read_dir(dir).expect("list img dir") {
            let entry = entry.expect("dir entry");
            if entry.file_name().to_string_lossy().starts_with("slab-") {
                p = Some(entry.path());
            }
        }
        p.expect("slab sidecar present")
    };
    let store = SlabStore::from_bytes(vec![std::fs::read(&slab_path).expect("read slab")])
        .expect("slab parses");
    let drop_id = *store.drop_index_keys().next().expect("one drop");
    assert_eq!(store.drop_is_seekable(&drop_id), Some(false));

    let reader = ImageReader::open(&img.path, ReadConfig::default()).expect("open");
    let mut file = reader.file("/big.bin").expect("file");
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut out).expect("read");
    assert_eq!(out, big);

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(dir);
}
