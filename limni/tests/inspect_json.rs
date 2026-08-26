//! Integration test: `limni inspect --json` parses and contains the
//! layered-aware fields. Lives in `tests/` so `CARGO_BIN_EXE_limni`
//! is set; the unit-test version inside `src/main.rs` cannot spawn
//! the binary.
use limnifs_write::{write_directory_with_config, WriteConfig};
use std::path::PathBuf;
fn build(src: &PathBuf) -> PathBuf {
    let _ = std::fs::remove_dir_all(src);
    std::fs::create_dir_all(src).expect("mkdir");
    std::fs::write(src.join("a.txt"), b"hello limnifs").expect("write");
    std::fs::write(src.join("b.txt"), vec![0u8; 50 * 1024]).expect("write");
    let mut config = WriteConfig::default_v0_1();
    config.defaults.text_codec = "lz4".into();
    config.defaults.binary_codec = "lz4".into();
    let art = write_directory_with_config(src, &config).expect("pack");
    let dir = std::env::temp_dir().join(format!("limni-inspect-json-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir img");
    std::fs::write(dir.join("image.lim"), &art.bytes).expect("manifest");
    for slab in &art.slabs {
        let name = limnifs_core::locator::local_sidecar_name(&slab.locator).expect("flat slab");
        std::fs::write(dir.join(name), &slab.bytes).expect("slab");
    }
    if let Some(side) = &art.metadata_sidecar {
        let name = limnifs_core::locator::local_sidecar_name(&side.locator).expect("flat sidecar");
        std::fs::write(dir.join(name), &side.bytes).expect("metadata");
    }
    let _ = std::fs::remove_dir_all(src);
    dir.join("image.lim")
}
#[test]
fn inspect_json_reports_standalone_image_fields() {
    let img =
        build(&std::env::temp_dir().join(format!("limni-inspect-src-{}", std::process::id())));
    let out = std::process::Command::new(std::env::var("CARGO_BIN_EXE_limni").expect("bin"))
        .args(["inspect", img.to_str().expect("utf8"), "--json"])
        .output()
        .expect("spawn inspect --json");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("inspect --json emits valid JSON");

    assert_eq!(v["versions"]["drop_store"], 1);

    assert_eq!(v["versions"]["metadata"], 1);

    assert_eq!(v["versions"]["manifest"], 1);

    assert!(v["metadata"]["inline"].as_bool().unwrap());

    assert_eq!(v["chain_depth"], 0, "standalone: chain_depth 0");

    assert!(v["base_root"].is_null(), "standalone: no base root");
    let history = v["history"].as_array().expect("history array");

    assert_eq!(history.len(), 1, "single Build entry");

    assert_eq!(history[0]["op"], "build");
    let slabs = v["slabs"].as_array().expect("slabs");

    assert_eq!(slabs.len(), 1);

    assert!(slabs[0]["plaintext_bytes"].as_u64().unwrap() > 0);
    let ratio = slabs[0]["ratio"].as_f64().unwrap();

    assert!(ratio > 0.0 && ratio <= 1.0, "ratio {ratio} should be ≤ 1");

    assert!(
        !v["limni_version"].as_str().unwrap().is_empty(),
        "version reported"
    );
}
