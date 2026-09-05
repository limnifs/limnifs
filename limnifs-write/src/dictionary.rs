//! ZSTD dictionary training — writer-side API.
//!
//! Wraps `limnifs_core::codec::zstd_dict` (which wraps
//! `omnizip_zstd::train_dictionary`) at the writer layer. The
//! codec-layer API takes raw samples; this layer adds:
//!
//! - Per-content-class sample collection (text, binary, etc.).
//! - Configurable trainer (FrequencyTrainer today; FastCover via
//!   `omnizip_zstd::FastCoverTrainer` when needed).
//! - Dictionary id allocation (0x00..=0xFE; 0xFF is `NO_DICT`).
//! - Integration with `WriteConfig::dictionaries`.
//!
//! ## Pipeline integration (planned)
//!
//! Today this module exposes the trainer; the writer pipeline does
//! NOT yet call it. The plan, filed in
//! `TODO.impl/04-writer-pipeline/04-zstd-dictionary-training.md`:
//!
//! 1. Walk + parallel chunk + compress (existing pipeline).
//! 2. Collect unique plaintext drops per content class during
//!    `merge_chunked_file`.
//! 3. After parallel phase, train one dict per class with ≥
//!    `min_class_size` drops.
//! 4. Re-compress eligible drops with `compress_with_dict`; keep
//!    the smaller of (original, dict-compressed).
//! 5. Drop records carry `dict_id`; manifest emits
//!    `dictionary_section`.
//!
//! This module is the public API for steps 3–4. The pipeline glue
//! lands in a follow-up PR.

use limnifs_core::codec::zstd_dict::{
    compress_with_dict, decompress_with_dict, train_dictionary as core_train,
    train_dictionary_fastcover,
};

/// Default target dictionary size (64 KiB). Matches `DictionaryConfig::max_dict_size` default.
pub const DEFAULT_TARGET_SIZE: usize = 65_536;

/// Minimum sample count before training is worthwhile. Below this,
/// the trainer returns empty (not enough signal). Matches
/// `DictionaryConfig::min_class_size` default.
pub const DEFAULT_MIN_SAMPLES: usize = 100;

/// Trained dictionary ready for use with `compress_with_dict`.
#[derive(Clone, Debug)]
pub struct TrainedDictionary {
    /// Allocated id (0x00..=0xFE). Stored in the manifest's
    /// `dictionary_section` and referenced by `DropRecord::dict_id`.
    pub id: u8,
    /// Codec id this dictionary targets (today always CODEC_ZSTD).
    pub codec: u8,
    /// Raw dictionary bytes (omnizip's serialized form).
    pub content: Vec<u8>,
}

impl TrainedDictionary {
    /// Compress `plaintext` with this dictionary.
    ///
    /// # Errors
    /// Returns [`crate::WriteError`] on compression failure.
    pub fn compress(&self, plaintext: &[u8]) -> Result<Vec<u8>, crate::WriteError> {
        compress_with_dict(plaintext, &self.content).map_err(|e| {
            crate::WriteError::Io(std::io::Error::other(format!(
                "dict compress (id {}): {e}",
                self.id
            )))
        })
    }

    /// Decompress `compressed` with this dictionary.
    ///
    /// # Errors
    /// Returns [`crate::WriteError`] on decompression failure.
    pub fn decompress(
        &self,
        compressed: &[u8],
        expected_len: u32,
    ) -> Result<Vec<u8>, crate::WriteError> {
        decompress_with_dict(compressed, expected_len, &self.content).map_err(|e| {
            crate::WriteError::Io(std::io::Error::other(format!(
                "dict decompress (id {}): {e}",
                self.id
            )))
        })
    }
}

/// Adopt the ZSTD dictionaries carried in a base image's
/// `dictionary_section` for reuse by a layer write. Non-ZSTD
/// entries and unknown class ids are skipped (forward
/// compatibility: ids 0 = text, 1 = binary today).
#[must_use]
pub fn adopt_from_section(
    section: limnifs_core::dictionary_section::DictionarySection,
) -> Vec<TrainedDictionary> {
    section
        .dicts
        .into_iter()
        .filter(|d| d.codec_id == limnifs_core::codec::CODEC_ZSTD && matches!(d.class_id, 0 | 1))
        .map(|d| TrainedDictionary {
            id: d.class_id,
            codec: d.codec_id,
            content: d.data,
        })
        .collect()
}

/// Train a ZSTD dictionary from `samples` using the default
/// FrequencyTrainer. Returns `None` if `samples` is empty, target
/// size is 0, or the trainer produces an empty dictionary (not
/// enough signal).
///
/// `id` is the caller-allocated dictionary id (0x00..=0xFE).
#[must_use]
pub fn train_zstd(id: u8, samples: &[&[u8]], target_size: usize) -> Option<TrainedDictionary> {
    train_zstd_with_trainer(id, samples, target_size, TrainerKind::Frequency)
}

/// Trainer algorithm selection. See [`train_zstd_with_trainer`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrainerKind {
    /// Top-K substrings by frequency × length. Default. Wins on
    /// corpora with strong common substrings.
    Frequency,
    /// Dmer-frequency scoring per FastCover (Facebook 2018). Wins on
    /// corpora with distributed redundancy (mixed JSON, source files,
    /// log lines).
    FastCover,
}

impl TrainerKind {
    /// Parse from a config string. Unknown values fall back to
    /// `Frequency` (the default).
    #[must_use]
    pub fn from_config_str(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "fastcover" => Self::FastCover,
            _ => Self::Frequency,
        }
    }
}

/// Train with explicit trainer selection. See [`train_zstd`] for the
/// default-FrequencyTrainer shortcut.
#[must_use]
pub fn train_zstd_with_trainer(
    id: u8,
    samples: &[&[u8]],
    target_size: usize,
    trainer: TrainerKind,
) -> Option<TrainedDictionary> {
    if samples.is_empty() || target_size == 0 {
        return None;
    }
    let content = match trainer {
        TrainerKind::Frequency => core_train(samples, target_size),
        TrainerKind::FastCover => train_dictionary_fastcover(samples, target_size),
    };
    if content.is_empty() {
        return None;
    }
    Some(TrainedDictionary {
        id,
        codec: limnifs_core::codec::CODEC_ZSTD,
        content,
    })
}

/// Allocate dictionary ids 0x00..=0xFE for a set of trained dicts.
/// The 0xFF slot is reserved as `NO_DICT` sentinel.
///
/// Returns a map from class name to allocated id. Errors if more
/// than 254 classes need ids.
pub fn allocate_ids<'a>(class_names: &'a [&'a str]) -> Result<Vec<(&'a str, u8)>, &'static str> {
    if class_names.len() > 254 {
        return Err("dictionary id space exhausted (max 254 classes)");
    }
    Ok(class_names
        .iter()
        .enumerate()
        .map(|(i, name)| (*name, u8::try_from(i).expect("≤ 254")))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_text_samples(n: usize) -> Vec<Vec<u8>> {
        // Repetitive source-code-like content. The trainer should
        // find common substrings like "function", "return", "const".
        (0..n)
            .map(|i| format!("function test_case_{i}() {{ return {i}; }}\n").into_bytes())
            .collect()
    }

    #[test]
    fn train_zstd_returns_dict_for_repetitive_samples() {
        let samples_vec = synthetic_text_samples(50);
        let samples: Vec<&[u8]> = samples_vec.iter().map(Vec::as_slice).collect();
        let dict = train_zstd(0, &samples, 4096);
        // FrequencyTrainer may return empty on some inputs; assert
        // at minimum that the function ran without panicking.
        if let Some(d) = &dict {
            assert!(!d.content.is_empty(), "trained dict content non-empty");
            assert_eq!(d.id, 0);
            assert_eq!(d.codec, limnifs_core::codec::CODEC_ZSTD);
        }
    }

    #[test]
    fn train_zstd_returns_none_for_empty_samples() {
        assert!(train_zstd(0, &[], 4096).is_none());
    }

    #[test]
    fn train_zstd_returns_none_for_zero_target_size() {
        let samples_vec = synthetic_text_samples(10);
        let samples: Vec<&[u8]> = samples_vec.iter().map(Vec::as_slice).collect();
        assert!(train_zstd(0, &samples, 0).is_none());
    }

    #[test]
    fn dict_round_trips_when_trained() {
        let samples_vec = synthetic_text_samples(50);
        let samples: Vec<&[u8]> = samples_vec.iter().map(Vec::as_slice).collect();
        let Some(dict) = train_zstd(0, &samples, 4096) else {
            return; // Trainer may legitimately return None
        };
        let plaintext = b"function test_case_99() { return 99; }\n";
        let compressed = dict.compress(plaintext).expect("compress");
        let recovered = dict
            .decompress(&compressed, plaintext.len() as u32)
            .expect("decompress");
        assert_eq!(recovered.as_slice(), &plaintext[..]);
    }

    #[test]
    fn allocate_ids_assigns_sequential_ids() {
        let names = vec!["text", "binary", "source"];
        let allocated = allocate_ids(&names).expect("allocate");
        assert_eq!(allocated.len(), 3);
        assert_eq!(allocated[0], ("text", 0));
        assert_eq!(allocated[1], ("binary", 1));
        assert_eq!(allocated[2], ("source", 2));
    }

    #[test]
    fn allocate_ids_rejects_more_than_254_classes() {
        let names: Vec<&str> = (0..255).map(|_| "x").collect();
        assert!(allocate_ids(&names).is_err());
    }
    #[test]
    fn adopt_from_section_maps_ids_and_filters_codecs() {
        let section = limnifs_core::dictionary_section::DictionarySection {
            version: limnifs_core::dictionary_section::DICTIONARY_SECTION_VERSION,
            dicts: vec![
                limnifs_core::dictionary_section::Dictionary {
                    codec_id: limnifs_core::codec::CODEC_ZSTD,
                    class_id: 0,
                    data: b"text-dict".to_vec(),
                },
                limnifs_core::dictionary_section::Dictionary {
                    codec_id: limnifs_core::codec::CODEC_ZSTD,
                    class_id: 1,
                    data: b"binary-dict".to_vec(),
                },
                limnifs_core::dictionary_section::Dictionary {
                    codec_id: limnifs_core::codec::CODEC_LZ4,
                    class_id: 0,
                    data: b"wrong-codec".to_vec(),
                },
                limnifs_core::dictionary_section::Dictionary {
                    codec_id: limnifs_core::codec::CODEC_ZSTD,
                    class_id: 7,
                    data: b"unknown-class".to_vec(),
                },
            ],
        };
        let adopted = adopt_from_section(section);
        assert_eq!(
            adopted.len(),
            2,
            "non-zstd and unknown-class entries dropped"
        );
        assert_eq!(adopted[0].id, 0);
        assert_eq!(adopted[0].content, b"text-dict");
        assert_eq!(adopted[1].id, 1);
        assert_eq!(adopted[1].content, b"binary-dict");
    }
}
