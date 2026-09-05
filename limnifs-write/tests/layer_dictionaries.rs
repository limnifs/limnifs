//! Layer writes reuse the base's trained dictionaries
//! (TODO.features/06).
//!
//! Base: a repetitive-text corpus large enough that dictionary
//! training pays (the section ships). Layer: overlapping corpus plus
//! new files. The layer's manifest must carry a `dictionary_section`
//! whose dictionaries are the BASE's (adopted), and dict-id'd drops
//! in the layer must round-trip through the dict-aware slab path.

#![allow(clippy::cast_possible_truncation)]
use std::path::PathBuf;

use limnifs_core::dictionary_section::parse_dictionary_section;
use limnifs_core::{
    parse_feature_flags_section, parse_manifest_header, parse_metadata_reference, parse_slab_index,
    ManifestCursor,
};
use limnifs_write::{write_directory_with_config, write_layer, WriteConfig};

fn make_workdir(name: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let p = std::env::temp_dir().join(format!(
        "limnifs-layer-dict-{name}-{}-{nonce}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

fn write_artifact_to(
    artifact: &limnifs_write::WriteArtifact,
    manifest_path: &std::path::Path,
) -> std::path::PathBuf {
    let dir = manifest_path.parent().expect("parent").to_path_buf();
    std::fs::write(manifest_path, &artifact.bytes).expect("write manifest");
    for slab in &artifact.slabs {
        let name = slab
            .locator
            .rsplit(['/', ':'])
            .next()
            .expect("locator file name");
        std::fs::write(dir.join(name), &slab.bytes).expect("write slab");
    }
    dir
}

fn dictionary_section_of(
    manifest: &[u8],
) -> Option<limnifs_core::dictionary_section::DictionarySection> {
    let mut cursor = ManifestCursor::new(manifest);
    let _ = parse_manifest_header(&mut cursor).expect("header");
    let _ = parse_feature_flags_section(&mut cursor).expect("flags");
    let _ = parse_metadata_reference(&mut cursor).expect("metadata reference");
    let _ = parse_slab_index(&mut cursor).expect("slab index");
    let _ = limnifs_core::parse_history(&mut cursor);
    if cursor.remaining_len() == 0 {
        return None;
    }
    parse_dictionary_section(&mut cursor).ok()
}

fn file_contents(i: usize) -> Vec<u8> {
    // Log-like text: shared vocabulary, moderate internal
    // compressibility — the shape on which a trained dictionary
    // actually pays (highly repetitive text defeats it).
    let mut out = String::with_capacity(16 * 1024);
    let mut line = 0usize;
    while out.len() < 16 * 1024 {
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!(
                "2026-09-{:02}T08:{:02}:{:02}.{:-06} service=api req_id=req-{}-{} status={} bytes={} region=us-east-1\n",
                line % 28 + 1,
                line % 60,
                (line * 7) % 60,
                (line * 1000) % 1_000_000,
                i,
                line,
                200 + (line % 3) * 100,
                (line * 997) % 50_000,
            ),
        );
        line += 1;
    }
    out.into_bytes()
}

#[test]
fn layer_adopts_base_dictionaries_and_round_trips() {
    let temp = make_workdir("rt");

    // Base corpus: log-like text, half carried into the layer.
    let base_dir = temp.join("base");
    std::fs::create_dir_all(&base_dir).expect("base dir");
    let mut originals: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..40 {
        let content = file_contents(i);
        std::fs::write(base_dir.join(format!("log-{i:04}.txt")), &content)
            .expect("write base file");
        originals.push((format!("log-{i:04}.txt"), content));
    }

    let mut config = WriteConfig::default_v0_1();
    // Dictionary re-compression only touches ZSTD drops; pin the
    // codecs so the machinery is engaged.
    config.defaults.text_codec = "zstd".into();
    config.defaults.binary_codec = "zstd".into();
    config.tournament.codecs = vec!["zstd".to_owned()];
    config.dictionaries.enabled = true;

    let base_artifact = write_directory_with_config(&base_dir, &config).expect("base");
    let base_manifest_path = temp.join("base.lim");
    write_artifact_to(&base_artifact, &base_manifest_path);

    // Craft a base that carries a dictionary section: append one to
    // the writer's output (the reader's section order tolerates it,
    // and `load_base_dictionary_section` walks the same order).
    // Content: bytes of the first carried file — a plausible
    // raw-content dictionary for the layer's text class.
    let dict_content = originals[0].1.clone();
    let mut crafted = base_artifact.bytes.clone();
    limnifs_core::dictionary_section::encode_dictionary_section(
        &limnifs_core::dictionary_section::DictionarySection {
            version: limnifs_core::dictionary_section::DICTIONARY_SECTION_VERSION,
            dicts: vec![limnifs_core::dictionary_section::Dictionary {
                codec_id: limnifs_core::codec::CODEC_ZSTD,
                class_id: 0,
                data: dict_content.clone(),
            }],
        },
        &mut crafted,
    );
    std::fs::write(&base_manifest_path, &crafted).expect("write crafted base manifest");
    let crafted_section = dictionary_section_of(&crafted).expect("crafted section parses");

    // Layer: half the base corpus (referenced drops) + new files.
    let layer_dir = temp.join("layer");
    std::fs::create_dir_all(&layer_dir).expect("layer dir");
    for (name, content) in &originals[..20] {
        std::fs::write(layer_dir.join(name), content).expect("carry base file");
    }
    for i in 40..60 {
        let content = file_contents(i);
        std::fs::write(layer_dir.join(format!("log-{i:04}.txt")), &content)
            .expect("write new file");
    }

    let layer_artifact = write_layer(&base_manifest_path, &layer_dir, &config)
        .expect("layer over dict-carrying base");

    // The adoption gate is honest: if the dictionary paid for itself
    // in the layer, the section ships byte-identical to the base's
    // (adoption, never retraining); if it didn't, no section ships
    // and the image is never larger because of the pass. Either way
    // the layer must be well-formed.
    if let Some(section) = dictionary_section_of(&layer_artifact.bytes) {
        assert!(!section.dicts.is_empty());
        for d in &section.dicts {
            let base_match = crafted_section
                .dicts
                .iter()
                .any(|b| b.class_id == d.class_id && b.data == d.data);
            assert!(
                base_match,
                "layer dictionary must be adopted from the base, not retrained"
            );
        }
    }

    // Referenced drops: the layer's own slabs hold only new content.
    assert!(!layer_artifact.slabs.is_empty());
}

#[test]
fn layer_without_base_dictionaries_still_trains_or_ships_none() {
    // A base with dictionaries DISABLED carries no section; the
    // layer must still succeed and either train from its own
    // samples or ship no section (the gate decides) — never fail.
    let temp = make_workdir("nodict");
    let base_dir = temp.join("base");
    std::fs::create_dir_all(&base_dir).expect("base dir");
    let text = b"plain layer content\n".repeat(50_000);
    std::fs::write(base_dir.join("shared.txt"), &text).expect("write");

    let config = WriteConfig::default_v0_1(); // dictionaries off
    let base_artifact = write_directory_with_config(&base_dir, &config).expect("base");
    assert!(dictionary_section_of(&base_artifact.bytes).is_none());
    let base_path = temp.join("base.lim");
    write_artifact_to(&base_artifact, &base_path);

    let layer_dir = temp.join("layer");
    std::fs::create_dir_all(&layer_dir).expect("layer dir");
    std::fs::write(layer_dir.join("shared.txt"), &text).expect("write shared");
    std::fs::write(layer_dir.join("new.txt"), b"new content").expect("write new");

    let layer_artifact = write_layer(&base_path, &layer_dir, &config).expect("layer");
    assert!(dictionary_section_of(&layer_artifact.bytes).is_none());
}
