//! Default categorizers that ship with the v0.1 writer.
//!
//! These are the categorizers that were hardcoded in
//! `limnifs-write/src/file_categorizer/registry.rs` before
//! `WriteConfig` existed. They are exposed here so users can
//! copy them in a custom config or extend them.

#![allow(clippy::module_name_repetitions)]

use crate::config::CategorizerConfig;

/// FLAC for PCM WAV/AIFF audio.
#[must_use]
pub fn pcm_audio() -> CategorizerConfig {
    CategorizerConfig {
        name: "pcm_audio".into(),
        extensions: vec!["wav".into(), "aiff".into(), "aif".into()],
        magic_bytes: vec![b'R', b'I', b'F', b'F'], // WAV "RIFF"
        codec: "flac".into(),
        max_size: None,
        enabled: true,
    }
}

/// Rice++ for FITS astronomical tables.
#[must_use]
pub fn fits() -> CategorizerConfig {
    CategorizerConfig {
        name: "fits".into(),
        extensions: vec!["fits".into(), "fit".into(), "fts".into()],
        magic_bytes: vec![b'S', b'I', b'M', b'P', b'L', b'E'], // FITS block keyword
        codec: "ricepp".into(),
        max_size: None,
        enabled: true,
    }
}

/// FSST+Brotli for delimited text.
#[must_use]
pub fn delimited_text() -> CategorizerConfig {
    CategorizerConfig {
        name: "delimited_text".into(),
        extensions: vec!["csv".into(), "tsv".into(), "json".into(), "jsonl".into(), "ndjson".into()],
        magic_bytes: vec![],
        codec: "fsst+brotli".into(),
        max_size: Some(65_536),
        enabled: true,
    }
}

/// BZip2 for `.bz2` archives.
#[must_use]
pub fn bzip2() -> CategorizerConfig {
    CategorizerConfig {
        name: "bzip2".into(),
        extensions: vec!["bz2".into(), "tbz2".into()],
        magic_bytes: vec![b'B', b'Z', b'h'], // BZip2 magic
        codec: "bzip2".into(),
        max_size: None,
        enabled: true,
    }
}

/// All v0.1 default categorizers.
#[must_use]
pub fn all_v0_1() -> Vec<CategorizerConfig> {
    vec![pcm_audio(), fits(), delimited_text(), bzip2()]
}
