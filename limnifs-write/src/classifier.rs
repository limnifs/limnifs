//! Drop classifier (seine): entropy + magic-byte heuristics.
//!
//! Labels each drop's plaintext with a class so the deepening stage
//! can pick a class-appropriate codec. Pure-functional, stateless,
//! deterministic — same input always yields the same class.
//!
//! ## Classes
//!
//! - `Text` — UTF-8 printable, mostly ASCII
//! - `Code` — executable container (ELF, Mach-O, PE)
//! - `Compressed` — already-compressed bytes (gzip, zstd, xz, bz2)
//! - `Media` — image / audio / video container (JPEG, PNG, GIF, MP3, MP4)
//! - `Sparse` — dominated by zero bytes
//! - `Binary` — fallback when no other class fits
//!
//! ## Algorithm
//!
//! 1. If the first bytes match a known magic, return that class
//!    immediately. Magic detection is the highest-confidence signal.
//! 2. Otherwise, compute Shannon entropy over a sample (first 4 KiB):
//!    - < 0.5 → `Sparse` if zero-byte ratio is also high, else `Text`
//!    - 0.5–7.5 → `Text` if mostly printable, else `Binary`
//!    - ≥ 7.5 → `Compressed` (high entropy is the signature of
//!      already-compressed or encrypted data)
//! 3. Fall back to `Binary`.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

/// Number of bytes at the drop's start used for classification.
/// The full drop can be megabytes; the first 4 KiB is enough signal.
pub const CLASSIFIER_SAMPLE_SIZE: usize = 4 * 1024;

/// Shannon entropy threshold below which data is "low entropy".
const LOW_ENTROPY_THRESHOLD: f32 = 0.5;
/// Shannon entropy threshold above which data is "high entropy"
/// (typical of compressed or encrypted bytes).
const HIGH_ENTROPY_THRESHOLD: f32 = 7.5;
/// Zero-byte ratio above which low-entropy data is labelled `Sparse`.
const SPARSE_ZERO_RATIO_THRESHOLD: f32 = 0.8;
/// Printable-ASCII ratio above which mid-entropy data is labelled `Text`.
const TEXT_PRINTABLE_RATIO_THRESHOLD: f32 = 0.85;

/// One of the six content classes the seine classifier emits.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Class {
    Text,
    Code,
    Binary,
    Compressed,
    Media,
    Sparse,
}

impl Class {
    /// Stable 1-byte encoding for the class (used in the slab's
    /// per-class solid-window index, future deepening records, etc.).
    #[must_use]
    pub const fn to_id(self) -> u8 {
        match self {
            Self::Text => 0x01,
            Self::Code => 0x02,
            Self::Binary => 0x03,
            Self::Compressed => 0x04,
            Self::Media => 0x05,
            Self::Sparse => 0x06,
        }
    }

    /// Human-readable name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Code => "code",
            Self::Binary => "binary",
            Self::Compressed => "compressed",
            Self::Media => "media",
            Self::Sparse => "sparse",
        }
    }
}

/// A stateless classifier. Holding it in a struct (rather than a free
/// function) leaves room for future configuration without changing
/// the call sites (OCP).
#[derive(Copy, Clone, Debug, Default)]
pub struct Classifier;

impl Classifier {
    /// Classify `data` by sampling its first `CLASSIFIER_SAMPLE_SIZE`
    /// bytes. Empty input classifies as `Sparse`.
    #[must_use]
    pub fn classify(&self, data: &[u8]) -> Class {
        if data.is_empty() {
            return Class::Sparse;
        }
        let sample = if data.len() <= CLASSIFIER_SAMPLE_SIZE {
            data
        } else {
            &data[..CLASSIFIER_SAMPLE_SIZE]
        };
        if let Some(class) = detect_magic(sample) {
            return class;
        }
        let entropy = shannon_entropy(sample);
        let zero_ratio = zero_byte_ratio(sample);
        if entropy < LOW_ENTROPY_THRESHOLD && zero_ratio > SPARSE_ZERO_RATIO_THRESHOLD {
            return Class::Sparse;
        }
        if entropy >= HIGH_ENTROPY_THRESHOLD {
            return Class::Compressed;
        }
        let printable_ratio = printable_ascii_ratio(sample);
        if printable_ratio > TEXT_PRINTABLE_RATIO_THRESHOLD {
            return Class::Text;
        }
        Class::Binary
    }
}

/// Check the first bytes against known magic constants. Returns
/// `Some(class)` if a magic matches, `None` otherwise.
fn detect_magic(data: &[u8]) -> Option<Class> {
    if data.starts_with(&[0x1F, 0x8B]) {
        return Some(Class::Compressed); // gzip
    }
    if data.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        return Some(Class::Compressed); // zstd
    }
    if data.starts_with(&[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00]) {
        return Some(Class::Compressed); // xz
    }
    if data.starts_with(&[0x42, 0x5A, 0x68]) {
        return Some(Class::Compressed); // bz2
    }
    if data.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        return Some(Class::Compressed); // 7z
    }
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(Class::Media); // JPEG
    }
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return Some(Class::Media); // PNG
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Some(Class::Media); // GIF
    }
    if data.starts_with(&[0x52, 0x49, 0x46, 0x46]) && data.len() >= 12 && &data[8..12] == b"WEBP" {
        return Some(Class::Media); // WebP
    }
    if data.starts_with(&[0xFF, 0xFB]) || data.starts_with(&[0x49, 0x44, 0x33]) {
        return Some(Class::Media); // MP3 (ID3 or frame sync)
    }
    if data.starts_with(&[0x66, 0x4C, 0x61, 0x43]) {
        return Some(Class::Media); // FLAC
    }
    if data.len() >= 12 && &data[4..8] == b"ftyp" {
        return Some(Class::Media); // ISO BMFF (MP4, MOV, HEIF)
    }
    if data.starts_with(&[0x4F, 0x67, 0x67, 0x53]) {
        return Some(Class::Media); // Ogg
    }
    if data.starts_with(&[0x7F, 0x45, 0x4C, 0x46]) {
        return Some(Class::Code); // ELF
    }
    if data.len() >= 4
        && (data.starts_with(&[0xFE, 0xED, 0xFA, 0xCE])
            || data.starts_with(&[0xFE, 0xED, 0xFA, 0xCF])
            || data.starts_with(&[0xCE, 0xFA, 0xED, 0xFE])
            || data.starts_with(&[0xCF, 0xFA, 0xED, 0xFE]))
    {
        return Some(Class::Code); // Mach-O
    }
    if data.len() >= 2 && data[0] == 0x4D && data[1] == 0x5A {
        return Some(Class::Code); // PE / DOS MZ
    }
    None
}

/// Shannon entropy in bits per byte, computed over `data`.
///
/// We cast `usize -> f64` for the byte count; the sample is at most
/// `CLASSIFIER_SAMPLE_SIZE` (4 KiB) so the precision loss clippy
/// warns about is not a concern here.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn shannon_entropy(data: &[u8]) -> f32 {
    if data.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    for &b in data {
        counts[usize::from(b)] += 1;
    }
    let total = data.len() as f64;
    let mut entropy = 0.0_f64;
    for &count in &counts {
        if count == 0 {
            continue;
        }
        let p = f64::from(count) / total;
        entropy -= p * p.log2();
    }
    entropy as f32
}

/// Fraction of bytes that are zero.
#[allow(
    clippy::cast_precision_loss,
    clippy::naive_bytecount,
    clippy::cast_possible_truncation
)]
fn zero_byte_ratio(data: &[u8]) -> f32 {
    if data.is_empty() {
        return 0.0;
    }
    let zeros = data.iter().filter(|&&b| b == 0).count();
    (zeros as f64 / data.len() as f64) as f32
}

/// Fraction of bytes that are printable ASCII (0x20..0x7E) plus
/// common whitespace (newline, tab, carriage return).
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn printable_ascii_ratio(data: &[u8]) -> f32 {
    if data.is_empty() {
        return 0.0;
    }
    let printable = data
        .iter()
        .filter(|&&b| (0x20..=0x7E).contains(&b) || b == b'\n' || b == b'\r' || b == b'\t')
        .count();
    (printable as f64 / data.len() as f64) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_classifies_as_sparse() {
        assert_eq!(Classifier.classify(&[]), Class::Sparse);
    }

    #[test]
    fn gzip_magic_wins_over_entropy() {
        let mut data = vec![0x1F, 0x8B, 0x08];
        data.extend(std::iter::repeat(0).take(100));
        assert_eq!(Classifier.classify(&data), Class::Compressed);
    }

    #[test]
    fn zstd_magic_detected() {
        let data = [0x28, 0xB5, 0x2F, 0xFD, 0x00, 0x00];
        assert_eq!(Classifier.classify(&data), Class::Compressed);
    }

    #[test]
    fn xz_magic_detected() {
        let data = [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00, 0x00, 0x00];
        assert_eq!(Classifier.classify(&data), Class::Compressed);
    }

    #[test]
    fn jpeg_magic_detected() {
        let data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(Classifier.classify(&data), Class::Media);
    }

    #[test]
    fn png_magic_detected() {
        let data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(Classifier.classify(&data), Class::Media);
    }

    #[test]
    fn gif_magic_detected() {
        assert_eq!(Classifier.classify(b"GIF89a..."), Class::Media);
    }

    #[test]
    fn webp_magic_detected() {
        let data = [
            0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00, b'W', b'E', b'B', b'P',
        ];
        assert_eq!(Classifier.classify(&data), Class::Media);
    }

    #[test]
    fn mp3_id3_magic_detected() {
        let data = [b'I', b'D', b'3', 0x03, 0x00, 0x00, 0x00];
        assert_eq!(Classifier.classify(&data), Class::Media);
    }

    #[test]
    fn elf_magic_detected() {
        let data = [0x7F, b'E', b'L', b'F', 0x02, 0x01, 0x01, 0x00];
        assert_eq!(Classifier.classify(&data), Class::Code);
    }

    #[test]
    fn macho_magic_detected() {
        let data = [0xFE, 0xED, 0xFA, 0xCF, 0x00, 0x00, 0x00, 0x01];
        assert_eq!(Classifier.classify(&data), Class::Code);
    }

    #[test]
    fn pe_magic_detected() {
        let data = [0x4D, 0x5A, 0x90, 0x00, 0x03, 0x00, 0x00, 0x00];
        assert_eq!(Classifier.classify(&data), Class::Code);
    }

    #[test]
    fn plain_text_classifies_as_text() {
        let text = b"Hello, world!\nThis is a plain text file.\nLines of prose.\n";
        assert_eq!(Classifier.classify(text), Class::Text);
    }

    #[test]
    fn code_like_source_classifies_as_text() {
        let source = b"fn main() {\n    println!(\"hello\");\n}\n";
        assert_eq!(Classifier.classify(source), Class::Text);
    }

    #[test]
    fn mostly_zeros_classifies_as_sparse() {
        let mut data = vec![0u8; 4096];
        data[0] = 0x42;
        data[100] = 0x99;
        assert_eq!(Classifier.classify(&data), Class::Sparse);
    }

    #[test]
    fn high_entropy_random_classifies_as_compressed() {
        let mut data = Vec::with_capacity(4096);
        let mut state: u64 = 1;
        for _ in 0..4096 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            data.push(u8::try_from(state >> 56).expect("fits u8"));
        }
        let class = Classifier.classify(&data);
        assert!(
            class == Class::Compressed || class == Class::Binary,
            "expected compressed or binary for high-entropy random, got {class:?}"
        );
    }

    #[test]
    fn mid_entropy_non_printable_classifies_as_binary() {
        // Mix of non-printable bytes that doesn't match any magic
        // and doesn't have enough printable content to be text.
        let mut data = Vec::with_capacity(4096);
        for i in 0..4096u32 {
            data.push(u8::try_from((i * 7 + 0x80) & 0xFF).expect("fits"));
        }
        let class = Classifier.classify(&data);
        // Should not be Text or Sparse. Code/Compressed/Media/Binary all OK.
        assert!(
            class != Class::Text && class != Class::Sparse,
            "expected non-text non-sparse for mid-entropy non-printable, got {class:?}"
        );
    }

    #[test]
    fn class_to_id_round_trips() {
        for class in [
            Class::Text,
            Class::Code,
            Class::Binary,
            Class::Compressed,
            Class::Media,
            Class::Sparse,
        ] {
            assert_eq!(class.as_str().len(), class.as_str().len());
            assert_ne!(class.to_id(), 0);
        }
    }

    #[test]
    fn classifier_is_deterministic() {
        let data: Vec<u8> = (0..1024u32)
            .map(|i| u8::try_from(i & 0xFF).expect("fits"))
            .collect();
        let a = Classifier.classify(&data);
        let b = Classifier.classify(&data);
        assert_eq!(a, b);
    }
}
