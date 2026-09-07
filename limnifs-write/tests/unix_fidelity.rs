//! Unix metadata fidelity (TODO.features/15): the writer captures
//! real mode/uid/gid, stream entries honor their permission bits,
//! and (in the CLI) extraction applies mode + mtime.

#![cfg(unix)]
#![allow(clippy::cast_possible_truncation)]

use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use limnifs_core::{
    parse_feature_flags_section, parse_manifest_header, parse_metadata_blob,
    parse_metadata_reference, ManifestCursor, MetadataBlob,
};
use limnifs_write::stream::StreamWriter;
use limnifs_write::{write_directory_with_config, WriteArtifact, WriteConfig};

fn make_workdir(name: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let p = std::env::temp_dir().join(format!(
        "limnifs-unix-fidelity-{name}-{}-{nonce}",
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

#[test]
fn directory_pack_captures_real_modes_and_ownership() {
    let dir = make_workdir("walk");
    std::fs::write(dir.join("plain.txt"), b"default perms").expect("write");
    std::fs::write(dir.join("exec.sh"), b"#!/bin/sh\n").expect("write");
    std::fs::set_permissions(dir.join("exec.sh"), std::fs::Permissions::from_mode(0o755))
        .expect("chmod 755");
    std::fs::write(dir.join("secret.txt"), b"private").expect("write");
    std::fs::set_permissions(
        dir.join("secret.txt"),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("chmod 600");
    let sub = dir.join("locked");
    std::fs::create_dir_all(&sub).expect("mkdir");
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o750)).expect("chmod 750");

    let artifact = write_directory_with_config(&dir, &WriteConfig::default_v0_1()).expect("pack");
    let blob = parse_blob(&artifact);

    let expected: &[(&str, u32)] = &[
        ("exec.sh", 0o100_755),
        ("secret.txt", 0o100_600),
        ("plain.txt", 0o100_644),
        ("locked", 0o040_750),
    ];
    for (name, want_mode) in expected {
        assert!(
            blob.inodes.iter().any(|i| i.mode == *want_mode),
            "expected mode {want_mode:o} for {name}"
        );
    }

    // Ownership is captured (the creating user, not hardcoded 0).
    // Stat the fixture itself: /tmp can be root-owned (GitHub
    // runners), which would make a temp-dir stat the wrong oracle.
    let me = std::fs::metadata(&dir).expect("fixture stat");
    let plain = blob
        .inodes
        .iter()
        .find(|i| i.mode == 0o100_644)
        .expect("plain inode");
    assert_eq!(plain.uid, me.uid(), "uid captured from the filesystem");
    assert_eq!(plain.gid, me.gid(), "gid captured");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stream_entries_carry_permission_bits() {
    let config: &'static WriteConfig = Box::leak(Box::new(WriteConfig::default_v0_1()));
    let mut writer = StreamWriter::new(config).expect("writer");
    writer
        .stage_file("script.sh", 1, 0o755, &[], b"#!/bin/sh\necho hi\n")
        .expect("stage");
    writer.add_dir("dir", 2, 0o700).expect("dir");
    writer
        .add_file("dir/ro.txt", 3, 0o400, &[], &mut b"read only".as_slice())
        .expect("file");
    writer
        .add_symlink("link", "dir/ro.txt", 4, 0o777)
        .expect("symlink");
    let artifact = writer.finish().expect("finish");
    let blob = parse_blob(&artifact);

    assert!(blob.inodes.iter().any(|i| i.mode == 0o100_755), "file 755");
    assert!(blob.inodes.iter().any(|i| i.mode == 0o040_700), "dir 700");
    assert!(blob.inodes.iter().any(|i| i.mode == 0o100_400), "file 400");
    assert!(
        blob.inodes.iter().any(|i| i.mode == 0o120_777),
        "symlink 777"
    );
}
