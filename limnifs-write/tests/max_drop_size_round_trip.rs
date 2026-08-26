//! Bounded-output drops (`defaults.max_drop_size`, TODO.sota-fs/04).
//!
//! A drop's plaintext is the unit of decode cost on every random
//! access. The cap forces whole-file paths (categorizer claims) to
//! fall back to `FastCDC` chunking above the limit, so the bound holds
//! by construction (EROFS fixed-output pclusters). `0` restores the
//! unbounded behavior; `skip_chunking` is exempt — whole-file IS the
//! max-write profile's speed contract.

use limnifs_core::read::{ImageReader, ReadConfig};
use limnifs_write::{write_directory_with_config, WriteConfig};

fn csv_fixture(bytes: usize) -> Vec<u8> {
    // High-entropy rows: near-repetitive text defeats FastCDC
    // boundary detection, and the point of the test is that the
    // capped claim FALLS THROUGH to real chunking.
    let mut state = 0x0123_4567_89AB_CDEFu64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut out = Vec::with_capacity(bytes);
    let mut i = 0u64;
    while out.len() < bytes {
        let row = format!(
            "{i},{:016x}{:016x},{:.3},active\n",
            next(),
            next(),
            (i % 4096) as f32 / 7.0
        );
        out.extend_from_slice(row.as_bytes());
        i += 1;
    }
    out.truncate(bytes);
    out
}

fn write_image(dir: &std::path::Path, cap: u32) -> limnifs_write::WriteArtifact {
    let mut config = WriteConfig::default_v0_1();
    config.defaults.max_drop_size = cap;
    config.categorizers = limnifs_write::config::defaults::all_v0_1();
    write_directory_with_config(dir, &config).expect("write")
}

fn materialize(art: &limnifs_write::WriteArtifact) -> std::path::PathBuf {
    let img = std::env::temp_dir().join(format!(
        "limnifs-max-drop-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos())
    ));
    std::fs::create_dir_all(&img).expect("mkdir img");
    std::fs::write(img.join("image.lim"), &art.bytes).expect("manifest");
    for slab in &art.slabs {
        let name =
            limnifs_core::locator::local_sidecar_name(&slab.locator).expect("flat slab name");
        std::fs::write(img.join(name), &slab.bytes).expect("slab");
    }
    if let Some(side) = &art.metadata_sidecar {
        let name =
            limnifs_core::locator::local_sidecar_name(&side.locator).expect("flat metadata name");
        std::fs::write(img.join(name), &side.bytes).expect("metadata sidecar");
    }
    img.join("image.lim")
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "limnifs-max-drop-src-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos())
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir src");
    dir
}

#[test]
fn capped_csv_falls_back_to_chunking_and_round_trips() {
    let original = csv_fixture(2 * 1024 * 1024);
    let src = scratch("capped");
    std::fs::write(src.join("data.csv"), &original).expect("write csv");

    // Cap 8 KiB: the 64 KiB CSV is claimed by csv-text but exceeds the
    // cap, so it must fall through to FastCDC chunking.
    let art = write_image(&src, 8 * 1024);
    assert!(
        art.drop_count > 1,
        "capped whole-file claim must chunk, got {} drops",
        art.drop_count
    );
    let image = materialize(&art);
    let reader = ImageReader::open(&image, ReadConfig::default()).expect("open");
    let mut file = reader.file("/data.csv").expect("file");
    assert_eq!(file.size(), original.len() as u64);
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut out).expect("read");
    assert_eq!(out, original, "capped image round-trips byte-exact");

    // Windowed access through the new reader as well.
    let file = reader.file("/data.csv").expect("file");
    let mut window = vec![0u8; 4096];
    let n = file.read_at(30_000, &mut window).expect("read_at");
    assert_eq!(&window[..n], &original[30_000..30_000 + n]);

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(image.parent().expect("img parent"));
}

#[test]
fn zero_cap_preserves_whole_file_behavior() {
    let original = csv_fixture(2 * 1024 * 1024);
    let src = scratch("zero");
    std::fs::write(src.join("data.csv"), &original).expect("write csv");

    // Cap 0 = unlimited: the csv-text claim stands and the whole file
    // is a single FSST+Brotli drop.
    let art = write_image(&src, 0);
    assert_eq!(art.drop_count, 1, "unlimited cap keeps the whole-file drop");
    let image = materialize(&art);
    let reader = ImageReader::open(&image, ReadConfig::default()).expect("open");
    let mut file = reader.file("/data.csv").expect("file");
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut out).expect("read");
    assert_eq!(out, original);

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(image.parent().expect("img parent"));
}

#[test]
fn skip_chunking_is_exempt_from_the_cap() {
    let original = csv_fixture(2 * 1024 * 1024);
    let src = scratch("skip");
    std::fs::write(src.join("blob.bin"), &original).expect("write blob");

    let mut config = WriteConfig::default_v0_1();
    config.skip_chunking = true;
    config.defaults.max_drop_size = 8 * 1024;
    let art = write_directory_with_config(&src, &config).expect("write");
    assert_eq!(art.drop_count, 1, "skip_chunking keeps whole-file drops");

    let image = materialize(&art);
    let reader = ImageReader::open(&image, ReadConfig::default()).expect("open");
    let mut file = reader.file("/blob.bin").expect("file");
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut out).expect("read");
    assert_eq!(out, original);

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(image.parent().expect("img parent"));
}

#[test]
fn validation_rejects_sub_kilobyte_caps() {
    let mut config = WriteConfig::default_v0_1();
    config.defaults.max_drop_size = 512;
    assert!(
        config.validate().is_err(),
        "nonzero cap below 1 KiB is rejected"
    );
    config.defaults.max_drop_size = 0;
    assert!(config.validate().is_ok(), "0 (unlimited) is accepted");
    config.defaults.max_drop_size = 4096;
    assert!(config.validate().is_ok());
}

#[test]
fn chunking_config_changes_emitted_chunk_sizes() {
    // F4 (TODO.sota-fs/09): the `[chunking]` section was parsed and
    // validated but never applied — the writer hardcoded
    // FastCDC::default(). It must now be honoured.
    let payload: Vec<u8> = (0..3 * 1024 * 1024)
        .map(|i: usize| ((i / 97) % 256) as u8 ^ ((i % 251) as u8))
        .collect();
    let src = scratch("chunkcfg");
    std::fs::write(src.join("data.bin"), &payload).expect("write");

    let small = {
        let mut config = WriteConfig::default_v0_1();
        config.chunking.avg_chunk_size = 16 * 1024;
        config.chunking.min_chunk_size = 4 * 1024;
        config.chunking.max_chunk_size = 64 * 1024;
        config.defaults.text_codec = "lz4".into();
        config.defaults.binary_codec = "lz4".into();
        write_directory_with_config(&src, &config).expect("small chunks")
    };
    let big = write_image(&src, 0);

    assert!(
        small.drop_count > big.drop_count,
        "smaller configured chunks must yield more drops: {} vs {}",
        small.drop_count,
        big.drop_count
    );

    // Round-trip the small-chunk image.
    let image = materialize(&small);
    let reader = ImageReader::open(&image, ReadConfig::default()).expect("open");
    let mut file = reader.file("/data.bin").expect("file");
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut out).expect("read");
    assert_eq!(out, payload);

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(image.parent().expect("img parent"));
}
