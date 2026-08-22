//! Regression for issue #186: images carrying deduplicated
//! shared-inline inodes must parse through the limnifs-core reader.
//! The reserved-flag mask previously covered the defined
//! `INODE_FLAG_SHARED_INLINE` bit, rejecting exactly these inodes.

use std::collections::HashMap;

use limnifs_core::{
    parse_feature_flags_section, parse_manifest_header, parse_metadata_blob,
    parse_metadata_reference, ContentHandle, ManifestCursor,
};
use limnifs_write::{write_directory_with_config, WriteConfig};

#[test]
fn duplicated_inline_files_round_trip_through_reader() {
    let dir = std::env::temp_dir().join(format!("limnifs-shared-inline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");

    // Distinct payloads, each written to 2-3 different paths so the
    // shared-inline dedup table is exercised with several entries.
    let payloads: Vec<Vec<u8>> = (0..4u8)
        .map(|i| vec![i.wrapping_mul(37).wrapping_add(11); 512])
        .collect();
    let mut copies: HashMap<usize, usize> = HashMap::new();
    for (i, p) in payloads.iter().enumerate() {
        for copy in 0..(2 + i % 2) {
            std::fs::write(dir.join(format!("payload{i}_copy{copy}.bin")), p).expect("write");
            *copies.entry(i).or_default() += 1;
        }
    }

    let art = write_directory_with_config(&dir, &WriteConfig::default_v0_1()).expect("write");
    assert_eq!(art.drop_count, 0, "fixture stays fully inline");

    // Parse exactly as a consumer would: mask rejection fires here.
    let mut c = ManifestCursor::new(&art.bytes);
    parse_manifest_header(&mut c).expect("header");
    parse_feature_flags_section(&mut c).expect("flags");
    let mr = parse_metadata_reference(&mut c).expect("metadata reference");
    let bytes = mr.inline_metadata.as_deref().expect("metadata inlined");
    let blob = parse_metadata_blob(&mut ManifestCursor::new(bytes)).expect("blob parses");

    let index = blob.build_path_index();
    let mut seen: HashMap<usize, usize> = HashMap::new();
    for (path, num) in &index {
        if !path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .starts_with("payload")
        {
            continue; // root directory inode
        }
        let inode = blob.inode_by_number(*num).expect("inode");
        let got = match &inode.content_handle {
            ContentHandle::InlineData(d) => d.clone(),
            other => panic!("unexpected content handle: {other:?}"),
        };
        let stem = path.rsplit('/').next().unwrap_or(path);
        let idx = stem
            .strip_prefix("payload")
            .and_then(|r| r.split('_').next())
            .and_then(|d| d.parse::<usize>().ok())
            .expect("fixture filename encodes payload index");
        assert_eq!(got, payloads[idx], "path {path} round trip");
        *seen.entry(idx).or_default() += 1;
    }
    assert_eq!(seen, copies, "every fixture file accounted for");
}

#[test]
fn shared_inline_flag_not_in_reserved_mask() {
    // Pins the contract: bit 3 is defined, so it must sit OUTSIDE the
    // reserved mask. 0xF8 here regresses issue #186.
    assert_eq!(limnifs_core::INODE_FLAG_RESERVED_MASK, 0xF0);
    assert_eq!(
        limnifs_core::INODE_FLAG_RESERVED_MASK & limnifs_core::INODE_FLAG_SHARED_INLINE,
        0,
        "SHARED_INLINE must not be covered by the reserved mask"
    );
}
