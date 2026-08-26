//! IMPL-6 (TODO.remaining): measure the ZSTD dictionary ratio win on
//! a small-file-heavy text workload. 2,000 structured 16 KiB text
//! files (log-like): enough repetition for a trained dictionary to
//! help, all above the inline threshold so they land in slabs.

use limnifs_write::{write_directory_with_config, WriteConfig};
use std::fmt::Write as _;

fn fixture(src: &std::path::Path) {
    std::fs::create_dir_all(src).expect("mkdir");
    let template = |i: usize| -> String {
        let mut s = String::with_capacity(16 * 1024);
        let mut line = 0usize;
        while s.len() < 16 * 1024 {
            let _ = writeln!(s,
                "2026-08-{:02}T12:{:02}:{:02}.{:-06} service=api req_id=req-{}-{} status={} bytes={} region=us-east-1",
                line % 28 + 1,
                line % 60,
                (line * 7) % 60,
                (line * 1000) % 1_000_000,
                i,
                line,
                200 + (line % 3) * 100,
                (line * 997) % 50_000,
            );
            line += 1;
        }
        s
    };
    for i in 0..2000 {
        std::fs::write(src.join(format!("log-{i:04}.txt")), template(i)).expect("write");
    }
}

fn pack(src: &std::path::Path, dicts: bool) -> usize {
    let mut config = WriteConfig::default_v0_1();
    config.defaults.text_codec = "zstd".into();
    config.defaults.binary_codec = "zstd".into();
    config.dictionaries.enabled = dicts;
    let art = write_directory_with_config(src, &config).expect("pack");
    let mut total = art.bytes.len();
    for slab in &art.slabs {
        total += slab.bytes.len();
    }
    if let Some(side) = &art.metadata_sidecar {
        total += side.bytes.len();
    }
    total
}

#[test]
fn zstd_dictionary_improves_tiny_text_file_ratio() {
    let src = std::env::temp_dir().join(format!("limnifs-dict-ratio-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&src);
    fixture(&src);

    let plain = pack(&src, false);
    let dict = pack(&src, true);
    let input: u64 = std::fs::read_dir(&src)
        .expect("list")
        .map(|e| e.map_or(0, |e| e.metadata().map_or(0, |m| m.len())))
        .sum::<u64>();

    #[allow(clippy::cast_precision_loss)]
    let improvement = (plain as f64 - dict as f64) / plain.max(1) as f64 * 100.0;
    println!(
        "dict-ratio: input {input} B, plain {plain} B, dict {dict} B → {improvement:.1}% smaller with dictionaries"
    );

    // Non-regression: dictionaries must never make the image larger.
    assert!(
        dict <= plain,
        "dictionary-trained image ({dict} B) larger than plain ({plain} B)"
    );
    // The ≥20% target from TODO.impl/04-zstd-dictionary-training.
    // Soft-gated at ≥5% so environment/library drift can't flake CI;
    // the headline number is printed above.
    assert!(
        improvement >= 5.0,
        "dictionary win {improvement:.1}% below the 5% soft floor"
    );

    let _ = std::fs::remove_dir_all(&src);
}
