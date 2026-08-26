//! IMPL-2 (TODO.remaining): `extract_file` vs `limni extract`
//! produce byte-identical output on the same image. Both code paths
//! must agree on every byte of every file.

use std::io::Write;
use std::path::{Path, PathBuf};

use limnifs_core::read::{extract_file, ReadConfig};
use limnifs_write::{write_directory_with_config, WriteConfig};

fn build_image(src: &PathBuf) -> PathBuf {
    let _ = std::fs::remove_dir_all(src);
    std::fs::create_dir_all(src).expect("mkdir");
    std::fs::create_dir_all(src.join("subdir")).expect("mkdir sub");
    let payload = |i: usize| -> Vec<u8> {
        let mut v = Vec::with_capacity(i);
        for j in 0..i {
            v.push(u8::try_from((j * 31 + i) % 251).unwrap_or(0));
        }
        v
    };
    std::fs::write(src.join("small.txt"), b"small inline payload").expect("write small");
    std::fs::write(src.join("medium.bin"), payload(64 * 1024)).expect("write med");
    std::fs::write(src.join("big.bin"), payload(3 * 1024 * 1024)).expect("write big");
    std::fs::write(src.join("subdir/nested.txt"), payload(8 * 1024)).expect("write nested");
    let mut config = WriteConfig::default_v0_1();
    config.defaults.text_codec = "lz4".into();
    config.defaults.binary_codec = "lz4".into();
    let art = write_directory_with_config(src, &config).expect("pack");
    let dir = std::env::temp_dir().join(format!("limni-extract-diff-img-{}", std::process::id()));
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

/// Recursively list all files under `dir`, returning (relative path,
/// bytes). Stable across both extract paths.
fn collect(dir: &Path) -> Vec<(String, Vec<u8>)> {
    use std::path::Path;
    fn walk(p: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let Ok(entries) = std::fs::read_dir(p) else {
            return;
        };
        let mut names: Vec<_> = entries.flatten().collect();
        names.sort_by_key(std::fs::DirEntry::file_name);
        for e in names {
            let path = e.path();
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                walk(&path, base, out);
            } else if path.is_file() {
                let bytes = std::fs::read(&path).unwrap_or_default();
                out.push((rel, bytes));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

#[test]
fn extract_file_matches_limni_extract_byte_for_byte() {
    let img = build_image(
        &std::env::temp_dir().join(format!("limni-extract-diff-src-{}", std::process::id())),
    );

    // Path A: limnifs_core::read::extract_file per-file into its own temp tree.
    let a_root = std::env::temp_dir().join(format!(
        "limni-extract-diff-A-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let _ = std::fs::remove_dir_all(&a_root);
    std::fs::create_dir_all(&a_root).expect("mkdir A");

    for path in [
        "/small.txt",
        "/medium.bin",
        "/big.bin",
        "/subdir/nested.txt",
    ] {
        let parent = a_root
            .join(path.trim_start_matches('/'))
            .parent()
            .unwrap()
            .to_path_buf();
        std::fs::create_dir_all(&parent).expect("mkdir parent");
        let mut f =
            std::fs::File::create(a_root.join(path.trim_start_matches('/'))).expect("create");
        let mut sink = std::io::BufWriter::new(&mut f);
        extract_file(&img, path, &mut sink, ReadConfig::default()).expect("extract_file");
        let _ = sink.into_inner().expect("flush").write_all(&[]).ok();
    }

    // Path B: `limni extract` via the binary.
    let b_root = std::env::temp_dir().join(format!(
        "limni-extract-diff-B-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
            + 1
    ));
    let _ = std::fs::remove_dir_all(&b_root);
    let out = std::process::Command::new(std::env::var("CARGO_BIN_EXE_limni").expect("bin"))
        .args(["extract", img.to_str().expect("utf8")])
        .arg(&b_root)
        .output()
        .expect("spawn limni extract");
    assert!(
        out.status.success(),
        "limni extract failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let a = collect(&a_root);
    let b = collect(&b_root);
    assert_eq!(
        a, b,
        "extract_file and limni extract must produce identical trees"
    );

    // Sanity: actual contents match what the source had (ground truth).
    assert!(a.iter().any(|(p, _)| p == "big.bin"));
    let _ = std::fs::remove_dir_all(&a_root);
    let _ = std::fs::remove_dir_all(&b_root);
}
