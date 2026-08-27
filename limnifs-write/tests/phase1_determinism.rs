//! Phase-1 parallel hashing must not change output: packing the
//! same tree twice yields byte-identical manifest and slabs.
//!
//! `process_file` hashes chunks via `par_iter` (work stealing inside
//! a rayon worker). Drop ids are per-chunk BLAKE3 values and the
//! indexed collect preserves chunk order, so the artifact must be
//! bit-for-bit reproducible. This test is the pin: if anyone changes
//! Phase 1 in a way that lets worker scheduling leak into the bytes
//! (ordering, dedup, slab layout), it fails.

use limnifs_write::write_directory;

fn build_tree(root: &std::path::Path) {
    std::fs::create_dir_all(root).expect("mkdir");
    // In-file duplicate chunks: a block slightly above the average
    // chunk size, repeated with a one-byte counter prefix so FastCDC
    // cuts several boundaries including repeats.
    let block: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let mut big = Vec::with_capacity(block.len() * 8);
    for run in 0..8u8 {
        big.push(run);
        big.extend_from_slice(&block);
    }
    std::fs::write(root.join("repeated.bin"), &big).expect("write repeated");
    // Cross-file duplicate: same bytes under a second name.
    std::fs::write(root.join("repeated-copy.bin"), &big).expect("write copy");
    // A small text file so the tree also exercises the inline path.
    std::fs::write(root.join("note.txt"), b"phase-1 determinism pin\n").expect("write note");
}

fn artifact_bytes(art: &limnifs_write::WriteArtifact) -> Vec<Vec<u8>> {
    let mut out = vec![art.bytes.clone()];
    out.extend(art.slabs.iter().map(|s| s.bytes.clone()));
    out
}

#[test]
fn packing_twice_is_byte_identical() {
    let base = std::env::temp_dir().join(format!("limnifs-det-{}", std::process::id()));
    let src = base.join("src");
    let _ = std::fs::remove_dir_all(&base);
    build_tree(&src);

    let first = write_directory(&src).expect("first pack");
    let second = write_directory(&src).expect("second pack");

    assert_eq!(
        artifact_bytes(&first),
        artifact_bytes(&second),
        "same tree must pack to byte-identical manifest + slabs"
    );

    let _ = std::fs::remove_dir_all(&base);
}
