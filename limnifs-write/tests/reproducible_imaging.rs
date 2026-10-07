//! Reproducible imaging (tamatebako/tebako#718): with the
//! reproducibility knobs on, imaging the same tree must produce
//! byte-identical output regardless of the host's mtimes, ownership,
//! or permission bits; with them off (and `SOURCE_DATE_EPOCH` unset),
//! real host metadata is still recorded.
//!
//! Semantics under test:
//!
//! - `WriteConfig::source_date_epoch` pins every recorded mtime to
//!   exactly that epoch (pin, not clamp). When the field is `None`,
//!   the `SOURCE_DATE_EPOCH` environment variable supplies the pin;
//!   a malformed value is a named error.
//! - `WriteConfig::normalize_metadata` zeroes uid/gid and
//!   canonicalizes permission bits (0o755 dirs / executables /
//!   0o777 symlinks, 0o644 other files; type and exec bit preserved).
//!
//! Environment discipline: exactly one test function below touches
//! `SOURCE_DATE_EPOCH`; every other test sets the config field
//! explicitly, which takes precedence over the environment, so the
//! tests cannot race each other inside this binary.

use std::path::{Path, PathBuf};

use limnifs_core::{
    parse_feature_flags_section, parse_manifest_header, parse_metadata_blob,
    parse_metadata_reference, ContentHandle, ManifestCursor, MetadataBlob,
};
use limnifs_write::{write_directory_with_config, WriteArtifact, WriteConfig};

const EPOCH_A: u64 = 1_577_836_800; // 2020-01-01T00:00:00Z
const EPOCH_B: u64 = 1_717_200_000; // 2024-04-01T00:00:00Z
const PIN: u64 = 1_700_000_000; // the recorded pin
const PIN_NS: u64 = PIN * 1_000_000_000;

fn make_workdir(name: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let p = std::env::temp_dir().join(format!(
        "limnifs-repro-{name}-{}-{nonce}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

/// One fixed tree. `old` selects the metadata generation: the two
/// stagings carry identical content but deliberately different mtimes
/// and permission bits (uid/gid cannot differ without root; the
/// normalize knob zeroes them, which the canonicalization test pins).
fn stage_tree(root: &Path, old: bool) {
    let mtime = if old { EPOCH_A } else { EPOCH_B };
    #[cfg(unix)]
    let (plain_mode, exec_mode, dir_mode) = if old {
        (0o644, 0o755, 0o755)
    } else {
        (0o600, 0o700, 0o700)
    };
    std::fs::create_dir_all(root.join("sub")).expect("mkdir sub");
    std::fs::write(root.join("note.txt"), b"reproducible me\n").expect("write note");
    // Big enough to leave the inline path and enter the slab path.
    let big: Vec<u8> = (0..9000u32).map(|i| (i % 233) as u8).collect();
    std::fs::write(root.join("sub").join("blob.bin"), &big).expect("write blob");
    std::fs::write(root.join("run.sh"), b"#!/bin/sh\nexit 0\n").expect("write run.sh");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(
            root.join("note.txt"),
            std::fs::Permissions::from_mode(plain_mode),
        )
        .expect("chmod note");
        std::fs::set_permissions(
            root.join("run.sh"),
            std::fs::Permissions::from_mode(exec_mode),
        )
        .expect("chmod run.sh");
        std::fs::set_permissions(root.join("sub"), std::fs::Permissions::from_mode(dir_mode))
            .expect("chmod sub");
        std::os::unix::fs::symlink("note.txt", root.join("note-link")).expect("symlink");
        // A hardlink pair: both names share one inode in the image.
        std::fs::hard_link(root.join("note.txt"), root.join("note-hardlink")).expect("hardlink");
    }

    // Pin host mtimes to the staging's generation so the two stagings
    // differ by years, not milliseconds. Best-effort on directories
    // (some platforms refuse directory futimens; file mtimes are the
    // controlled variable and creation-time skew covers the rest).
    let stamp = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(mtime);
    for entry in ["note.txt", "run.sh", "sub/blob.bin"] {
        let f = std::fs::File::options()
            .write(true)
            .open(root.join(entry))
            .expect("open for set_modified");
        f.set_modified(stamp).expect("set_modified");
    }
    for d in ["sub", ""] {
        if let Ok(f) = std::fs::File::open(root.join(d)) {
            let _ = f.set_modified(stamp);
        }
    }
}

fn artifact_bytes(art: &WriteArtifact) -> Vec<Vec<u8>> {
    let mut out = vec![art.bytes.clone()];
    out.extend(art.slabs.iter().map(|s| s.bytes.clone()));
    if let Some(sidecar) = &art.metadata_sidecar {
        out.push(sidecar.bytes.clone());
    }
    out
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

fn reproducible_config() -> WriteConfig {
    WriteConfig::default_v0_1()
        .with_source_date_epoch(PIN)
        .with_normalized_metadata()
}

#[test]
fn deterministic_mode_is_byte_identical_across_stagings() {
    let base = make_workdir("stagings");
    let tree_a = base.join("checkout-a");
    let tree_b = base.join("checkout-b");
    stage_tree(&tree_a, true);
    stage_tree(&tree_b, false);
    // Let the directory mtimes of the two stagings diverge naturally
    // as well (entry creation touches them after the explicit stamp).
    std::fs::write(tree_a.join("note.txt"), b"reproducible me\n").expect("rewrite note a");
    std::fs::write(tree_b.join("note.txt"), b"reproducible me\n").expect("rewrite note b");

    let config = reproducible_config();
    let first = write_directory_with_config(&tree_a, &config).expect("pack a");
    let second = write_directory_with_config(&tree_b, &config).expect("pack b");

    assert_eq!(
        artifact_bytes(&first),
        artifact_bytes(&second),
        "identical content with different host mtime/mode must image byte-identically"
    );
    assert_eq!(first.merkle_root, second.merkle_root);

    // Every recorded mtime is exactly the pin — never the host's.
    let blob = parse_blob(&first);
    // Unix stagings add a symlink + hardlink; non-unix trees stop at
    // root + subdir + 3 files.
    #[cfg(unix)]
    let min_inodes = 6;
    #[cfg(not(unix))]
    let min_inodes = 5;
    assert!(
        blob.inodes.len() >= min_inodes,
        "expected at least {min_inodes} inodes, got {}",
        blob.inodes.len()
    );
    for inode in &blob.inodes {
        assert_eq!(
            inode.mtime_ns, PIN_NS,
            "inode {} must carry the pinned epoch",
            inode.number
        );
    }

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn normalize_metadata_canonicalizes_identity() {
    use limnifs_core::inode::{S_IFDIR, S_IFLNK, S_IFMT, S_IFREG};

    let base = make_workdir("canon");
    let tree = base.join("tree");
    stage_tree(&tree, false); // 0o600 file, 0o700 exec, 0o700 dir

    let artifact = write_directory_with_config(&tree, &reproducible_config()).expect("pack");
    let blob = parse_blob(&artifact);

    let inline_note = blob
        .inodes
        .iter()
        .find(|i| matches!(&i.content_handle, ContentHandle::InlineData(d) if d == b"reproducible me\n"))
        .expect("note.txt inode");
    assert_eq!(inline_note.mode & S_IFMT, S_IFREG);
    assert_eq!(
        inline_note.mode & 0o7777,
        0o644,
        "0o600 normalizes to 0o644"
    );
    assert_eq!((inline_note.uid, inline_note.gid), (0, 0));

    let exec = blob
        .inodes
        .iter()
        .find(|i| matches!(&i.content_handle, ContentHandle::InlineData(d) if d == b"#!/bin/sh\nexit 0\n"))
        .expect("run.sh inode");
    assert_eq!(
        exec.mode & 0o7777,
        0o755,
        "0o700 normalizes to 0o755 — the exec bit survives, owner variance does not"
    );

    let dir = blob
        .inodes
        .iter()
        .find(|i| {
            i.mode & S_IFMT == S_IFDIR && matches!(i.content_handle, ContentHandle::Directory(_))
        })
        .expect("a directory inode");
    assert_eq!(dir.mode & 0o7777, 0o755, "0o700 dir normalizes to 0o755");

    let link = blob
        .inodes
        .iter()
        .find(|i| i.mode & S_IFMT == S_IFLNK)
        .expect("symlink inode");
    assert_eq!(link.mode & 0o7777, 0o777, "symlinks canonicalize to 0o777");

    // The hardlink pair shares one inode with nlink 2.
    assert_eq!(inline_note.nlink, 2, "hardlink pair shares the inode");

    let _ = std::fs::remove_dir_all(&base);
}

/// The only test in this binary that touches `SOURCE_DATE_EPOCH`;
/// the others pin the config field explicitly (which wins over the
/// environment), so nothing here races.
#[test]
fn source_date_epoch_env_precedence_and_default_mode() {
    let base = make_workdir("env");
    let tree = base.join("tree");
    stage_tree(&tree, true);
    let note = tree.join("note.txt");

    // 1. Env honored when the config field is unset.
    std::env::set_var("SOURCE_DATE_EPOCH", "1700000000");
    let art = write_directory_with_config(&tree, &WriteConfig::default_v0_1()).expect("pack env");
    let blob = parse_blob(&art);
    for inode in &blob.inodes {
        assert_eq!(
            inode.mtime_ns, PIN_NS,
            "inode {} must carry the env-supplied epoch",
            inode.number
        );
    }

    // 2. The config field wins over the environment.
    let cfg = WriteConfig::default_v0_1().with_source_date_epoch(42);
    let art = write_directory_with_config(&tree, &cfg).expect("pack field");
    let blob = parse_blob(&art);
    for inode in &blob.inodes {
        assert_eq!(inode.mtime_ns, 42_000_000_000, "config field beats env");
    }

    // 3. A malformed value is a named error, never a silent fallback.
    std::env::set_var("SOURCE_DATE_EPOCH", "not-a-number");
    let err = write_directory_with_config(&tree, &WriteConfig::default_v0_1())
        .expect_err("malformed SOURCE_DATE_EPOCH must fail");
    assert!(
        err.to_string().contains("SOURCE_DATE_EPOCH"),
        "error must name the variable: {err}"
    );

    // 4. Negative control: no knob, no env → real host mtimes are
    //    recorded, and a host-mtime change changes the image bytes.
    std::env::remove_var("SOURCE_DATE_EPOCH");
    let real_ns = {
        let m = std::fs::symlink_metadata(&note)
            .expect("stat")
            .modified()
            .expect("mtime")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("post-1970");
        m.as_secs() * 1_000_000_000 + u64::from(m.subsec_nanos())
    };
    let art_t1 =
        write_directory_with_config(&tree, &WriteConfig::default_v0_1()).expect("pack default t1");
    let blob = parse_blob(&art_t1);
    let note_inode = blob
        .inodes
        .iter()
        .find(|i| matches!(&i.content_handle, ContentHandle::InlineData(d) if d == b"reproducible me\n"))
        .expect("note.txt inode");
    assert_eq!(
        note_inode.mtime_ns, real_ns,
        "default mode must record the host's real mtime"
    );

    let stamp_b = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(EPOCH_B);
    std::fs::File::options()
        .write(true)
        .open(&note)
        .expect("open")
        .set_modified(stamp_b)
        .expect("retouch");
    let art_t2 =
        write_directory_with_config(&tree, &WriteConfig::default_v0_1()).expect("pack default t2");
    assert_ne!(
        artifact_bytes(&art_t1),
        artifact_bytes(&art_t2),
        "default mode: different host mtimes must yield different images"
    );

    let _ = std::fs::remove_dir_all(&base);
}
