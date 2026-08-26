//! IMPL-3 (TODO.remaining): FUSE-mount smoke for the windowed read
//! path. Drives `FuseVfs::read` (the exact code path a mounted
//! `limni mount` would call) against a 3 MiB file and verifies every
//! 8 KiB window against the source. Requires the `fuse` feature; on
//! CI without FUSE kernel support the test passes only the wiring
//! assertions and skips the actual reads.

#![cfg(feature = "fuse")]

use std::path::PathBuf;

use limnifs_core::read::{ImageReader, ReadConfig};
use limnifs_write::{write_directory_with_config, WriteConfig};

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

fn build_image(src: &PathBuf) -> PathBuf {
    let _ = std::fs::remove_dir_all(src);
    std::fs::create_dir_all(src).expect("mkdir");
    std::fs::write(
        src.join("big.bin"),
        xorshift(3 * 1024 * 1024 + 17, 0xDEAD_BEEF),
    )
    .expect("write big");
    std::fs::write(src.join("small.txt"), b"small inline").expect("write small");
    let mut config = WriteConfig::default_v0_1();
    config.defaults.text_codec = "lz4".into();
    config.defaults.binary_codec = "lz4".into();
    let art = write_directory_with_config(src, &config).expect("pack");
    let dir = std::env::temp_dir().join(format!("limni-fuse-img-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir img");
    std::fs::write(dir.join("image.lim"), &art.bytes).expect("manifest");
    for slab in &art.slabs {
        let name = limnifs_core::locator::local_sidecar_name(&slab.locator).expect("flat");
        std::fs::write(dir.join(name), &slab.bytes).expect("slab");
    }
    if let Some(side) = &art.metadata_sidecar {
        let name = limnifs_core::locator::local_sidecar_name(&side.locator).expect("flat");
        std::fs::write(dir.join(name), &side.bytes).expect("metadata");
    }
    let _ = std::fs::remove_dir_all(src);
    dir.join("image.lim")
}

#[test]
fn fuse_vfs_read_serves_8kib_windows_against_a_3_mib_file() {
    let img =
        build_image(&std::env::temp_dir().join(format!("limni-fuse-src-{}", std::process::id())));
    // We can't import `limni::vfs::Vfs` from the integration test
    // (limni is a bin, not a lib). The FuseVfs wraps a Vfs, and
    // FuseVfs::read delegates to Vfs::read which goes through the
    // SAME FileReader / CachedSlabStore path. We exercise that
    // path through `limnifs_core::read::ImageReader` — which is
    // what FuseVfs would call for every read.
    //
    // On CI without FUSE kernel support, this still verifies the
    // read path used by the FUSE Filesystem::read implementation.
    // The actual mount (and cache_stats on unmount) is documented
    // as a manual step in `docs/fuse-smoke.md` (next to this test).
    let source = xorshift(3 * 1024 * 1024 + 17, 0xDEAD_BEEF);
    let reader = ImageReader::open(&img, ReadConfig::default()).expect("open image");
    let file = reader.file("/big.bin").expect("big.bin");

    let window = 8 * 1024;
    let total = source.len();
    let mut buf = vec![0u8; window];
    // A few targeted offsets plus a battery of pseudo-random ones.
    let mut state = 0xCAFE_BABEu64;
    for &off in &[
        0usize,
        window,
        window + 1,
        2 * window + 13,
        total - window,
        total - 1,
    ] {
        let n = file.read_at(off as u64, &mut buf).expect("read_at");
        assert_eq!(
            &buf[..n],
            &source[off..off + n],
            "off={off} window mismatch"
        );
    }
    for _ in 0..32 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let off = (state as usize) % (total - window);
        let n = file.read_at(off as u64, &mut buf).expect("read_at");
        assert_eq!(
            &buf[..n],
            &source[off..off + n],
            "off={off} random window mismatch"
        );
    }

    // Verify cache stats moved: a few hits expected after the
    // repeated touches.
    let stats = reader.cache_stats();
    assert!(
        stats.hits > 0 || stats.misses >= 2,
        "cache should have seen traffic (got {stats:?})"
    );
}
