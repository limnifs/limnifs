//! Xattr fidelity (TODO.features/17): the wire format has carried
//! an xattr block since v0 — the writer now emits it (feature
//! `xattr`), sorted and capped, with volatile platform namespaces
//! filtered; the CLI reapplies on extract.

#![cfg(all(unix, feature = "xattr"))]
#![allow(clippy::cast_possible_truncation)]

use std::path::PathBuf;

use limnifs_core::{
    parse_feature_flags_section, parse_manifest_header, parse_metadata_blob,
    parse_metadata_reference, ManifestCursor,
};
use limnifs_write::{write_directory_with_config, WriteConfig};

fn make_workdir(name: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0u64, |d| d.as_millis() as u64);
    let p = std::env::temp_dir().join(format!(
        "limnifs-xattrs-{name}-{}-{nonce}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

fn parse_inode_for(blob_bytes: &[u8], want_key: &str) -> limnifs_core::Inode {
    let mut cursor = ManifestCursor::new(blob_bytes);
    parse_manifest_header(&mut cursor).expect("header");
    parse_feature_flags_section(&mut cursor).expect("flags");
    let meta_ref = parse_metadata_reference(&mut cursor).expect("metadata reference");
    let inline = meta_ref
        .inline_metadata
        .as_ref()
        .expect("test trees stay inline");
    let mut blob_cursor = ManifestCursor::new(inline);
    let blob = parse_metadata_blob(&mut blob_cursor).expect("blob");
    blob.inodes
        .iter()
        .find(|i| i.xattrs.iter().any(|x| x.key == want_key))
        .cloned()
        .expect("inode carrying the xattr")
}

#[test]
fn xattrs_are_captured_sorted_and_deterministic() {
    let dir = make_workdir("capture");
    std::fs::write(dir.join("tagged.txt"), b"xattr carrier").expect("write");
    xattr::set(dir.join("tagged.txt"), "user.zeta", b"z").expect("set zeta");
    xattr::set(dir.join("tagged.txt"), "user.alpha", b"a-value").expect("set alpha");
    xattr::set(dir.join("tagged.txt"), "user.beta", b"b").expect("set beta");

    let artifact = write_directory_with_config(&dir, &WriteConfig::default_v0_1()).expect("pack");
    let inode = parse_inode_for(&artifact.bytes, "user.alpha");

    let keys: Vec<&str> = inode.xattrs.iter().map(|x| x.key.as_str()).collect();
    assert_eq!(
        keys,
        vec!["user.alpha", "user.beta", "user.zeta"],
        "name-sorted"
    );
    let alpha = inode.xattrs.iter().find(|x| x.key == "user.alpha").unwrap();
    assert_eq!(alpha.namespace, 0);
    assert_eq!(alpha.value, b"a-value");

    // Deterministic: same tree (same attrs) -> same bytes.
    let again =
        write_directory_with_config(&dir, &WriteConfig::default_v0_1()).expect("pack again");
    assert_eq!(artifact.bytes, again.bytes);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_without_xattrs_carry_no_block() {
    let dir = make_workdir("plain");
    std::fs::write(dir.join("plain.txt"), b"no attrs").expect("write");
    let artifact = write_directory_with_config(&dir, &WriteConfig::default_v0_1()).expect("pack");
    let mut cursor = ManifestCursor::new(&artifact.bytes);
    parse_manifest_header(&mut cursor).expect("header");
    parse_feature_flags_section(&mut cursor).expect("flags");
    let meta_ref = parse_metadata_reference(&mut cursor).expect("metadata reference");
    let inline = meta_ref.inline_metadata.as_ref().expect("inline");
    let mut blob_cursor = ManifestCursor::new(inline);
    let blob = parse_metadata_blob(&mut blob_cursor).expect("blob");
    assert!(
        blob.inodes.iter().all(|i| i.xattrs.is_empty()),
        "no xattr blocks on a plain tree"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
