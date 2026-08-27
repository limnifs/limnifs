//! IMPL-7 (TODO.remaining item 1): cross-image sparse index wiring.
//! A layer built over a base with a `<base>.sparse` Bloom sidecar
//! must produce BYTE-IDENTICAL output to the exact-HashSet path —
//! the Bloom only moves work, never answers.
//!
//! Feature-gated on `sparse-index` (matches the wiring).

#![cfg(feature = "sparse-index")]

use std::path::{Path, PathBuf};

use limnifs_write::{emit_sparse_sidecar, write_directory_with_config, write_layer, WriteConfig};

fn build_base(temp: &Path) -> PathBuf {
    let base_dir = temp.join("base");
    std::fs::create_dir_all(&base_dir).expect("base dir");
    let text = b"layer test content line\n".repeat(50_000); // ~1.15 MiB
    std::fs::write(base_dir.join("shared.txt"), &text).expect("write shared");
    std::fs::write(base_dir.join("base-only.txt"), b"only in base").expect("write base-only");

    let artifact =
        write_directory_with_config(&base_dir, &WriteConfig::default_v0_1()).expect("base");
    let manifest = temp.join("base.lim");
    std::fs::write(&manifest, &artifact.bytes).expect("base manifest");
    for slab in &artifact.slabs {
        let name = limnifs_core::locator::local_sidecar_name(&slab.locator).expect("flat");
        std::fs::write(temp.join(name), &slab.bytes).expect("base slab");
    }
    manifest
}

fn build_layer_tree(temp: &Path) -> PathBuf {
    let layer_dir = temp.join("layer");
    std::fs::create_dir_all(&layer_dir).expect("layer dir");
    let text = b"layer test content line\n".repeat(50_000);
    std::fs::write(layer_dir.join("shared.txt"), &text).expect("write shared");
    std::fs::write(layer_dir.join("new.txt"), b"only in the layer").expect("write new");
    layer_dir
}

#[test]
fn bloom_backed_layer_output_is_byte_identical_to_exact() {
    let temp = std::env::temp_dir().join(format!(
        "limnifs-sparse-wiring-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir_all(&temp).expect("temp dir");

    let base = build_base(&temp);
    let layer_tree = build_layer_tree(&temp);
    let config = WriteConfig::default_v0_1();

    // Exact path (no sidecar present).
    let exact = write_layer(&base, &layer_tree, &config).expect("exact layer");

    // Emit the BASE's Bloom sidecar (indexes the base's drop ids),
    // then take the sparse-backed path.
    let base_dir = temp.join("base");
    let base_artifact = write_directory_with_config(&base_dir, &config).expect("re-base");
    emit_sparse_sidecar(&base_artifact, &base).expect("base sidecar");
    assert!(
        base.with_extension("lim.sparse").is_file(),
        "sidecar written"
    );

    let bloom = write_layer(&base, &layer_tree, &config).expect("bloom layer");

    // Byte-identical: manifest and every slab.
    assert_eq!(exact.bytes, bloom.bytes, "manifest identical");
    assert_eq!(exact.slabs.len(), bloom.slabs.len());
    for (a, b) in exact.slabs.iter().zip(&bloom.slabs) {
        assert_eq!(a.bytes, b.bytes, "slab identical");
    }

    // And the layer actually deduped (not vacuously identical by
    // storing everything): the layer's slab must be small — just
    // the new file.
    let layer_bytes: usize = bloom.slabs.iter().map(|s| s.bytes.len()).sum();
    assert!(
        layer_bytes < 100_000,
        "layer slab carries only the new file, got {layer_bytes} bytes"
    );

    let _ = std::fs::remove_dir_all(&temp);
}
