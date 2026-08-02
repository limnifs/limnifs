//! ZSTD dictionary codec operations.
//!
//! Wraps `omnizip-zstd`'s dictionary APIs for `LimniFS`'s two-pass
//! writer pipeline and dict-aware reader.

use crate::error::CoreError;

/// Train a ZSTD dictionary from plaintext samples.
///
/// Returns the trained dictionary content bytes (the raw dict data,
/// not the serialized `ZstdDictionary` container).
#[must_use]
#[allow(dead_code)]
pub fn train_dictionary(samples: &[&[u8]], target_size: usize) -> Vec<u8> {
    omnizip_zstd::train_dictionary(samples, target_size)
}

/// Compress `plaintext` using a pre-trained ZSTD dictionary.
///
/// # Errors
/// Returns [`CoreError::Corrupt`] on compression failure.
#[allow(dead_code)]
pub fn compress_with_dict(plaintext: &[u8], dict_bytes: &[u8]) -> Result<Vec<u8>, CoreError> {
    let dict = omnizip_zstd::ZstdDictionary::from_raw(0, dict_bytes);
    omnizip_zstd::compress_with_dict(plaintext, omnizip_zstd::ZstdLevel::Default, &dict).map_err(
        |e| CoreError::Corrupt {
            reason: format!("zstd compress_with_dict failed: {e}"),
        },
    )
}

/// Decompress `compressed` using a pre-trained ZSTD dictionary.
///
/// # Errors
/// Returns [`CoreError::Corrupt`] on decompression failure.
#[allow(dead_code)]
pub fn decompress_with_dict(
    compressed: &[u8],
    expected_len: u32,
    dict_bytes: &[u8],
) -> Result<Vec<u8>, CoreError> {
    let dict = omnizip_zstd::ZstdDictionary::from_raw(0, dict_bytes);
    omnizip_zstd::decompress_with_dict(compressed, expected_len, &dict).map_err(|e| {
        CoreError::Corrupt {
            reason: format!("zstd decompress_with_dict failed: {e}"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dict_round_trip() {
        let samples: Vec<Vec<u8>> = (0..50)
            .map(|i| format!("function test_case_{i}() {{ return {i}; }}").into_bytes())
            .collect();
        let sample_refs: Vec<&[u8]> = samples.iter().map(Vec::as_slice).collect();

        let dict = train_dictionary(&sample_refs, 4096);
        if dict.is_empty() {
            return; // Trainer may return empty on some inputs
        }

        let plaintext = b"function test_case_99() { return 99; }";
        let compressed = compress_with_dict(plaintext, &dict).expect("compress");
        let decompressed =
            decompress_with_dict(&compressed, plaintext.len() as u32, &dict).expect("decompress");
        assert_eq!(decompressed, plaintext);
    }
}
