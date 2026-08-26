//! Regression tests for limnifs#195 and limnifs#196 (v0.3.0).
//!
//! #195: the `FastCDC` chunk path never emitted seekable containers —
//! per-chunk flags were hardcoded `0`, and even with the emission
//! function called, the 1 MiB strict threshold could never fire for
//! chunks bounded by `max_chunk_size` = 1 MiB.
//!
//! #196: `[[categorizers]]` config entries were never consulted —
//! only their presence enabled the static built-in registry, so a
//! user routing `.bin` files to a codec got chunked, flag-`0` output.

use limnifs_core::seekable::DROP_FLAG_SEEKABLE;
use limnifs_write::config::CategorizerConfig;
use limnifs_write::{write_directory_with_config, WriteConfig};

/// Parse drop records out of the produced slab; returns
/// `(plaintext_len, codec, flags)` per drop in declaration order.
fn drop_records(art: &limnifs_write::WriteArtifact) -> Vec<(u32, u8, u8)> {
    let slab = &art.slabs[0].bytes;
    let view = limnifs_core::parse_slab(slab).expect("slab parses");
    view.drop_records()
        .iter()
        .map(|r| (r.plaintext_len, r.representation.codec, r.flags))
        .collect()
}

#[test]
fn default_config_emits_seekable_drops_on_the_chunk_path() {
    // limnifs#195: 3 MiB compressible file through
    // `WriteConfig::default_v0_1()` (categorizers EMPTY) must yield
    // chunk drops with DROP_FLAG_SEEKABLE set.
    let src = std::env::temp_dir().join(format!("limnifs-195-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(&src).expect("mkdir");
    // Text-like and compressible so the classifier routes to the
    // text codec (cycling counter bytes classify as Media/Compressed
    // and short-circuit to STORE, which containers exclude).
    let mut payload = String::with_capacity(3 * 1024 * 1024);
    let mut line = 0usize;
    while payload.len() < 3 * 1024 * 1024 {
        use std::fmt::Write as _;
        let _ = writeln!(
            payload,
            "2026-08-26 service=api line={line} status={} bytes={}",
            200 + (line % 3) * 100,
            (line * 997) % 50_000
        );
        line += 1;
    }
    let payload = payload.into_bytes();
    std::fs::write(src.join("large.bin"), &payload).expect("write");

    let mut config = WriteConfig::default_v0_1();
    config.dictionaries.enabled = false;
    config.defaults.text_codec = "lz4".into();
    config.defaults.binary_codec = "lz4".into();
    let art = write_directory_with_config(&src, &config).expect("pack");

    let drops = drop_records(&art);
    assert!(
        drops
            .iter()
            .any(|(_, _, flags)| flags & DROP_FLAG_SEEKABLE != 0),
        "expected at least one seekable drop on the chunk path, got {drops:?}"
    );
    // Every chunk > 256 KiB (SEEKABLE_CHUNK_EMISSION_THRESHOLD) with
    // a general codec must be flagged.
    for (len, codec, flags) in &drops {
        let _ = codec;
        if *len as usize > limnifs_core::seekable::SEEKABLE_FRAME_SIZE {
            assert!(
                flags & DROP_FLAG_SEEKABLE != 0,
                "chunk of {len} bytes should be a container, flags={flags}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&src);
}

#[test]
fn config_categorizer_entries_route_whole_file_drops() {
    // limnifs#196: a `[[categorizers]]` entry routing `.bin` files
    // to lz4 must produce ONE whole-file drop for a 3 MiB file —
    // not FastCDC chunks. And with a general codec at 3 MiB >
    // SEEKABLE_EMISSION_THRESHOLD, the drop must be a container.
    let src = std::env::temp_dir().join(format!("limnifs-196-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(&src).expect("mkdir");
    let payload: Vec<u8> = b"the quick brown fox jumps over the lazy dog\n"
        .iter()
        .cycle()
        .take(3 * 1024 * 1024)
        .copied()
        .collect();
    std::fs::write(src.join("large.bin"), &payload).expect("write");

    let mut config = WriteConfig::default_v0_1();
    config.dictionaries.enabled = false;
    config.defaults.max_drop_size = 0; // no cap
    config.categorizers.push(CategorizerConfig {
        name: "bin-whole-file".into(),
        extensions: vec!["bin".into()],
        magic_bytes: Vec::new(),
        codec: "lz4".into(),
        max_size: None,
        enabled: true,
    });
    let art = write_directory_with_config(&src, &config).expect("pack");

    let drops = drop_records(&art);
    assert_eq!(
        drops.len(),
        1,
        "config categorizer must claim the file: expected 1 whole-file drop, got {drops:?}"
    );
    let (len, _codec, flags) = drops[0];
    assert_eq!(len as usize, payload.len(), "whole-file plaintext");
    assert!(
        flags & DROP_FLAG_SEEKABLE != 0,
        "3 MiB general-codec drop must be a seekable container, flags={flags}"
    );
    let _ = std::fs::remove_dir_all(&src);
}
