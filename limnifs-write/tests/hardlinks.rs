//! Hardlinks (TODO.features/18): the tree format shares inodes
//! across directory entries and carries nlink — the writer now uses
//! it. A second (dev, ino) occurrence becomes a metadata reference,
//! and stream/tar paths share inodes explicitly.

#![cfg(unix)]
#![allow(clippy::cast_possible_truncation)]

use std::path::PathBuf;

use limnifs_core::ContentHandle;
use limnifs_core::{
    parse_feature_flags_section, parse_manifest_header, parse_metadata_blob,
    parse_metadata_reference, ManifestCursor, MetadataBlob,
};
use limnifs_write::stream::{EntryMeta, StreamWriter};
use limnifs_write::{write_directory_with_config, WriteArtifact, WriteConfig};

fn make_workdir(name: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0u64, |d| d.as_millis() as u64);
    let p = std::env::temp_dir().join(format!(
        "limnifs-hardlinks-{name}-{}-{nonce}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

fn parse_blob(artifact: &WriteArtifact) -> MetadataBlob {
    let mut cursor = ManifestCursor::new(&artifact.bytes);
    parse_manifest_header(&mut cursor).expect("header");
    parse_feature_flags_section(&mut cursor).expect("flags");
    let meta_ref = parse_metadata_reference(&mut cursor).expect("metadata reference");
    let inline = meta_ref
        .inline_metadata
        .as_ref()
        .expect("test trees stay inline");
    let mut blob_cursor = ManifestCursor::new(inline);
    parse_metadata_blob(&mut blob_cursor).expect("blob")
}

/// Both names in the root directory node resolve to ONE inode,
/// and that inode carries `nlink == 2`.
#[test]
fn directory_pack_shares_hardlink_inodes() {
    let dir = make_workdir("walk");
    std::fs::write(dir.join("data.bin"), vec![0x5Au8; 300_000]).expect("write");
    std::fs::hard_link(dir.join("data.bin"), dir.join("alias.bin")).expect("link");

    let artifact = write_directory_with_config(&dir, &WriteConfig::default_v0_1()).expect("pack");
    let blob = parse_blob(&artifact);

    // Root dir node: two entries -> same inode number.
    let root = blob
        .inodes
        .iter()
        .find(|i| i.is_directory())
        .expect("root dir");
    let root_hash = match &root.content_handle {
        ContentHandle::Directory(h) => *h,
        _ => panic!("root has no dir hash"),
    };
    let node = blob.dir_node_by_hash(&root_hash).expect("dir node");
    let by_name: std::collections::HashMap<&str, u64> = node
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.inode_number))
        .collect();
    assert_eq!(by_name.len(), 2, "both names present");
    let data = by_name["data.bin"];
    assert_eq!(data, by_name["alias.bin"], "both names share one inode");

    let inode = blob
        .inodes
        .iter()
        .find(|i| i.number == data)
        .expect("shared inode");
    assert_eq!(inode.nlink, 2, "nlink reflects both names");
    assert_eq!(
        blob.inodes.len(),
        2,
        "root + one file inode; no duplicate for the link"
    );

    // Determinism: same tree -> same bytes (links resolve in walk
    // order every time).
    let again =
        write_directory_with_config(&dir, &WriteConfig::default_v0_1()).expect("pack again");
    assert_eq!(artifact.bytes, again.bytes);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Stream path: `add_hardlink` shares the staged entry's inode.
#[test]
fn stream_hardlink_shares_inode() {
    let config: &'static WriteConfig = Box::leak(Box::new(WriteConfig::default_v0_1()));
    let data = vec![0x11u8; 600 * 1024];
    let mut writer = StreamWriter::new(config).expect("writer");
    writer
        .stage_file("orig.bin", EntryMeta::new(1, 0o644), &[], &data)
        .expect("stage");
    writer.add_hardlink("link.bin", "orig.bin").expect("link");
    let artifact = writer.finish().expect("finish");
    let blob = parse_blob(&artifact);

    let root = blob
        .inodes
        .iter()
        .find(|i| i.is_directory())
        .expect("root dir");
    let hash = match &root.content_handle {
        limnifs_core::ContentHandle::Directory(h) => *h,
        _ => panic!("root dir"),
    };
    let node = blob.dir_node_by_hash(&hash).expect("dir node");
    let orig = node
        .entries
        .iter()
        .find(|e| e.name == "orig.bin")
        .expect("orig");
    let link = node
        .entries
        .iter()
        .find(|e| e.name == "link.bin")
        .expect("link");
    assert_eq!(orig.inode_number, link.inode_number, "shared inode");
    let inode = blob
        .inodes
        .iter()
        .find(|i| i.number == orig.inode_number)
        .expect("inode");
    assert_eq!(inode.nlink, 2);

    // Bad targets are rejected.
    let mut w = StreamWriter::new(config).expect("writer");
    assert!(w.add_hardlink("x", "missing").is_err());
    w.add_dir("d", EntryMeta::new(1, 0o755)).expect("dir");
    assert!(w.add_hardlink("x", "d").is_err(), "directory target");
}
