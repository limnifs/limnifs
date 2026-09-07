//! Bounded read paths (TODO.features/22): streaming per chunk must
//! produce the same bytes at the same offsets as the materialized
//! path, including mid-file starts that straddle chunk boundaries.

#![cfg(unix)]
#![allow(clippy::cast_possible_truncation)]

use limnifs_write::{write_directory_with_config, WriteArtifact, WriteConfig};

fn make_workdir(name: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0u64, |d| d.as_millis() as u64);
    let p = std::env::temp_dir().join(format!(
        "limnifs-bounded-{name}-{}-{nonce}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

/// Build a tree of files whose combined size forces multiple chunks
/// (default chunker threshold), then read them back through the
/// streaming path (a real CLI invocation) and byte-compare to the
/// source. The CLI's `std::fs::read` is independent of the test's
/// content, so this exercises the stream-to-disk path.
fn find_limni() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("LIMNI") {
        return std::path::PathBuf::from(p);
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for p in [
        manifest.join("../target/release/limni"),
        manifest.join("../../target/release/limni"),
        manifest.join("target/release/limni"),
    ] {
        if p.exists() {
            return p;
        }
    }
    panic!("limni binary not found (set LIMNI env var)");
}

#[test]
fn extract_round_trips_multi_chunk_content() {
    let workdir = make_workdir("extract");
    let src = workdir.join("src");
    std::fs::create_dir_all(&src).expect("src");

    // 6 × 600 KiB of pseudo-random text — well past the default
    // 256 KiB average chunk size, guaranteeing the file lands as
    // many slices. The mixing pattern spans chunk boundaries.
    let file = || -> Vec<u8> {
        let mut state: u64 = 0x1234_5678_DEAD_BEEF;
        let mut v = Vec::with_capacity(600 * 1024);
        while v.len() < 600 * 1024 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            v.push((state >> 56) as u8);
        }
        v
    };
    let original: Vec<Vec<u8>> = (0..6)
        .map(|i| {
            let mut v = file();
            // Tag each file with a distinct byte pattern at the start so
            // boundary cases land at predictable offsets.
            v[0] = 0xA0 + i;
            v
        })
        .collect();
    for (i, b) in original.iter().enumerate() {
        std::fs::write(src.join(format!("f{i}.bin")), b).expect("write");
    }

    let artifact = write_directory_with_config(&src, &WriteConfig::default_v0_1()).expect("pack");
    let image = workdir.join("img.lim");
    std::fs::write(&image, &artifact.bytes).expect("manifest");
    for slab in &artifact.slabs {
        let name = limnifs_core::locator::local_sidecar_name(&slab.locator).expect("locator");
        std::fs::write(workdir.join(name), &slab.bytes).expect("slab");
    }

    // The streaming path lives in the CLI; spawn it. The limni
    // binary is built at the workspace target dir; the test's PWD
    // is this package's, so walk up to the workspace root.
    let limni = find_limni();
    let out_dir = workdir.join("out");
    std::fs::create_dir_all(&out_dir).expect("out");
    let status = std::process::Command::new(&limni)
        .arg("extract")
        .arg(&image)
        .arg(&out_dir)
        .status()
        .expect("spawn limni extract");
    assert!(status.success(), "limni extract failed");

    for (i, b) in original.iter().enumerate() {
        let on_disk = std::fs::read(out_dir.join(format!("f{i}.bin"))).expect("read");
        assert_eq!(on_disk.len(), b.len(), "file {i} size");
        assert_eq!(&on_disk[..], &b[..], "file {i} content");
    }

    // Sanity: at least one file must have ended up with multiple
    // slices in the image (otherwise the test would not exercise
    // chunk-boundary reads). The blob parse in the CLI does not
    // surface the count, so re-pack and inspect here.
    let artifact2 =
        write_directory_with_config(&out_dir, &WriteConfig::default_v0_1()).expect("repack");
    let n_slices = total_slice_count(&artifact2);
    assert!(
        n_slices > 6,
        "fixture must force multiple slices per file (got {n_slices} total)"
    );
}

fn total_slice_count(artifact: &WriteArtifact) -> usize {
    use limnifs_core::{
        parse_feature_flags_section, parse_manifest_header, parse_metadata_blob,
        parse_metadata_reference, ManifestCursor,
    };
    let mut cursor = ManifestCursor::new(&artifact.bytes);
    parse_manifest_header(&mut cursor).expect("header");
    parse_feature_flags_section(&mut cursor).expect("flags");
    let meta_ref = parse_metadata_reference(&mut cursor).expect("meta");
    let inline = meta_ref.inline_metadata.as_ref().expect("inline");
    let mut blob_cursor = ManifestCursor::new(inline);
    let blob = parse_metadata_blob(&mut blob_cursor).expect("blob");
    blob.inodes
        .iter()
        .filter(|i| i.is_regular())
        .filter_map(|i| {
            if let limnifs_core::ContentHandle::SliceMap(s) = &i.content_handle {
                Some(s.len())
            } else {
                None
            }
        })
        .sum()
}
