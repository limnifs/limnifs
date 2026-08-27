use std::collections::HashMap;

use limnifs_core::{
    parse_feature_flags_section, parse_manifest_header, parse_metadata_blob,
    parse_metadata_reference, ContentHandle, ManifestCursor, S_IFLNK,
};
use limnifs_write::{write_directory_with_config, WriteConfig};
// Only the socket test below matches on WriteError, and it is
// unix-only (UnixListener) — ungated, this import warns on Windows.
#[cfg(unix)]
use limnifs_write::WriteError;

fn load_blob(bytes: &[u8]) -> limnifs_core::MetadataBlob {
    let mut c = ManifestCursor::new(bytes);
    parse_manifest_header(&mut c).expect("header");
    parse_feature_flags_section(&mut c).expect("flags");
    let mr = parse_metadata_reference(&mut c).expect("meta ref");
    let blob = mr.inline_metadata.as_deref().expect("inlined");
    parse_metadata_blob(&mut ManifestCursor::new(blob)).expect("blob")
}

#[test]
fn symlinks_round_trip_issue_190() {
    let dir = std::env::temp_dir().join(format!("limnifs-symlink-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".github/skills")).expect("mkdir");
    std::fs::create_dir_all(dir.join(".claude")).expect("mkdir");
    std::fs::write(dir.join(".github/skills/frontend.md"), b"skill one").expect("write");
    std::fs::write(dir.join("app.rb"), b"code").expect("write");
    // The git-gem shape: .claude/skills -> ../.github/skills.
    #[cfg(unix)]
    std::os::unix::fs::symlink("../.github/skills", dir.join(".claude/skills")).expect("link");
    // Dangling link: must still walk (symlink_metadata does not follow).
    #[cfg(unix)]
    std::os::unix::fs::symlink("no/such/target", dir.join("dangling")).expect("dangling");

    let art = write_directory_with_config(&dir, &WriteConfig::default_v0_1()).expect("write");
    let blob = load_blob(&art.bytes);

    let mut links: HashMap<String, String> = HashMap::new();
    for inode in &blob.inodes {
        if let ContentHandle::Symlink(target) = &inode.content_handle {
            assert_eq!(inode.mode & 0xF000, S_IFLNK, "symlink mode bits");
            links.insert(inode.number.to_string(), target.clone());
        }
    }
    #[cfg(unix)]
    {
        assert_eq!(links.len(), 2, "both links recorded: {links:?}");
        assert!(
            links.values().any(|t| t == "../.github/skills"),
            "in-tree relative target kept: {links:?}"
        );
        assert!(
            links.values().any(|t| t == "no/such/target"),
            "dangling target kept: {links:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn unsupported_types_raise_the_named_error_issue_190() {
    use std::os::unix::net::UnixListener;
    let dir = std::env::temp_dir().join(format!("limnifs-fifo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let socket_path = dir.join("sock");
    let _listener = UnixListener::bind(&socket_path).expect("bind socket");

    let err = write_directory_with_config(&dir, &WriteConfig::default_v0_1())
        .expect_err("socket must be rejected");
    match &err {
        WriteError::UnsupportedFileType { path, kind } => {
            assert_eq!(kind, "socket");
            assert!(path.ends_with("sock"), "{path:?}");
            let msg = err.to_string();
            assert!(msg.contains("unsupported file type"), "{msg}");
            assert!(
                msg.contains("symlinks"),
                "guidance mentions symlinks: {msg}"
            );
        }
        other @ WriteError::Io(_) => panic!("expected UnsupportedFileType, got {other:?}"),
    }
}
