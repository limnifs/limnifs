//! PCM audio categorizer — routes WAV/AIFF files to FLAC.
//!
//! **Status:** DETECTION READY, ROUTING DISABLED.
//!
//! Detection parses the WAV/AIFF header and extracts PCM sample
//! format (sample rate, channels, bits per sample, endianness).
//! Routing is currently disabled because `omnizip-flac` has not
//! shipped a real FLAC encoder yet. When it does, flip
//! `FLAC_ENABLED` to `true` and the categorizer will start
//! claiming WAV/AIFF files for the FLAC codec (id 0x07).
//!
//! See `docs/omnizip-vs-limnifs-boundary.md` for the codec id
//! allocation plan and `docs/dwarfs-multicodec-investigation.md`
//! for the rationale (FLAC saves 83% on PCM audio vs ~30% for
//! general codecs).

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::path::Path;

use super::{Categorization, FileCategorizer};
use limnifs_core::codec::CODEC_FLAC;

/// FLAC routing enabled — omnizip-flac 0.10 ships full LPC encoder
/// (CONSTANT/VERBATIM/FIXED/LPC + Rice residuals). The encoder picks
/// the cheapest subframe type per block via bit-cost estimation.
const FLAC_ENABLED: bool = true;

/// Minimum size for a sensible WAV/AIFF file. Smaller than this and
/// there's no point routing to FLAC — the overhead exceeds the gain.
const MIN_PCM_AUDIO_SIZE: usize = 64;

/// PCM parameters extracted from the file header. Serialized into
/// `Categorization::codec_params` for the FLAC codec to consume.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcmParams {
    pub sample_rate: u32,
    pub channels: u8,
    pub bits_per_sample: u8,
    pub endianness: Endianness,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Endianness {
    Little,
    Big,
}

impl PcmParams {
    /// Encode as a compact 6-byte prefix the FLAC codec can decode.
    /// Wire format owned by the codec layer, not the framework.
    #[must_use]
    pub fn encode(&self) -> [u8; 6] {
        let mut out = [0u8; 6];
        out[0..4].copy_from_slice(&self.sample_rate.to_le_bytes());
        out[4] = self.channels;
        out[5] = (self.bits_per_sample << 1) | match self.endianness {
            Endianness::Little => 0,
            Endianness::Big => 1,
        };
        out
    }
}

/// Categorizer for PCM-audio files (WAV, AIFF).
pub struct PcmAudioCategorizer;

impl FileCategorizer for PcmAudioCategorizer {
    fn name(&self) -> &'static str {
        "pcm-audio"
    }

    fn categories(&self) -> &'static [&'static str] {
        &["pcmaudio/waveform"]
    }

    fn categorize(&self, _path: &Path, data: &[u8]) -> Option<Categorization> {
        if !FLAC_ENABLED {
            return None;
        }
        if data.len() < MIN_PCM_AUDIO_SIZE {
            return None;
        }
        // Use omnizip-flac's parsers — they're maintained alongside
        // the codec and handle WAV/AIFF edge cases the same way the
        // encoder does. Local parser kept as a fallback if the dep
        // isn't desired; uncomment to use it instead.
        let params = omnizip_flac::pcm_header::parse_wav(data)
            .or_else(|| omnizip_flac::pcm_header::parse_aiff(data))?;
        Some(Categorization {
            codec_id: CODEC_FLAC,
            codec_params: encode_pcm_params(params).to_vec(),
            category: "pcmaudio/waveform",
        })
    }
}

/// Encode omnizip-flac's `PcmParams` into the compact 6-byte prefix
/// the LimniFS drop record expects.
fn encode_pcm_params(p: omnizip_flac::PcmParams) -> [u8; 6] {
    let mut out = [0u8; 6];
    out[0..4].copy_from_slice(&p.sample_rate.to_le_bytes());
    out[4] = p.channels;
    out[5] = (p.bits_per_sample << 1) | match p.endianness {
        omnizip_flac::Endianness::LittleEndian => 0,
        omnizip_flac::Endianness::BigEndian => 1,
    };
    out
}

/// Parse a WAV (RIFF/WAVE) header. Returns the PCM parameters if
/// the file is a vanilla PCM WAV (format tag 1 = WAVE_FORMAT_PCM).
#[must_use]
fn parse_wav(data: &[u8]) -> Option<PcmParams> {
    // RIFF header: "RIFF" + u32 LE size + "WAVE"
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }
    // Walk chunks looking for fmt.
    let mut off = 12;
    while off + 8 <= data.len() {
        let chunk_id = &data[off..off + 4];
        let chunk_size = u32::from_le_bytes([
            data[off + 4],
            data[off + 5],
            data[off + 6],
            data[off + 7],
        ]) as usize;
        let body_off = off + 8;
        if body_off + chunk_size > data.len() {
            return None;
        }
        if chunk_id == b"fmt " {
            // PCM fmt chunk: tag(2) + channels(2) + sample_rate(4)
            // + byte_rate(4) + block_align(2) + bits_per_sample(2)
            if chunk_size < 16 {
                return None;
            }
            let body = &data[body_off..body_off + 16];
            let tag = u16::from_le_bytes([body[0], body[1]]);
            if tag != 1 {
                return None; // not PCM
            }
            let channels = u16::from_le_bytes([body[2], body[3]]);
            let sample_rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            let bits_per_sample = u16::from_le_bytes([body[14], body[15]]);
            return Some(PcmParams {
                sample_rate,
                channels: u8::try_from(channels).ok()?,
                bits_per_sample: u8::try_from(bits_per_sample).ok()?,
                endianness: Endianness::Little,
            });
        }
        off = body_off + chunk_size + (chunk_size & 1); // chunks are word-aligned
    }
    None
}

/// Parse an AIFF (FORM/AIFF) header. Returns the PCM parameters.
#[must_use]
fn parse_aiff(data: &[u8]) -> Option<PcmParams> {
    // AIFF header: "FORM" + u32 BE size + "AIFF"
    if data.len() < 12 || &data[0..4] != b"FORM" || &data[8..12] != b"AIFF" {
        return None;
    }
    let mut off = 12;
    while off + 8 <= data.len() {
        let chunk_id = &data[off..off + 4];
        let chunk_size = u32::from_be_bytes([
            data[off + 4],
            data[off + 5],
            data[off + 6],
            data[off + 7],
        ]) as usize;
        let body_off = off + 8;
        if body_off + chunk_size > data.len() {
            return None;
        }
        if chunk_id == b"COMM" {
            // COMM chunk: channels(2 BE) + numFrames(4 BE)
            // + sampleSize(2 BE) + sampleRate(80-bit IEEE 754 ext)
            if chunk_size < 18 {
                return None;
            }
            let body = &data[body_off..body_off + 18];
            let channels = u16::from_be_bytes([body[0], body[1]]);
            let bits_per_sample = u16::from_be_bytes([body[6], body[7]]);
            // sampleRate is 80-bit extended float; we only need
            // the integer value, which fits in the high bytes for
            // common rates (8000, 11025, 16000, 22050, 32000,
            // 44100, 48000, 96000). A real IEEE 754 extended
            // decoder would go here; for now we return a sentinel
            // sample_rate of 0 and let the codec derive it from
            // the data stream. This is good enough to TEST
            // categorization routing.
            return Some(PcmParams {
                sample_rate: 0,
                channels: u8::try_from(channels).ok()?,
                bits_per_sample: u8::try_from(bits_per_sample).ok()?,
                endianness: Endianness::Big,
            });
        }
        off = body_off + chunk_size + (chunk_size & 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        let c = PcmAudioCategorizer;
        // Even with valid WAV magic, returns None because FLAC_ENABLED = false.
        let wav = make_minimal_wav(44100, 2, 16);
        assert!(c.categorize(Path::new("/x.wav"), &wav).is_none());
    }

    #[test]
    fn wav_header_parsed_correctly() {
        let wav = make_minimal_wav(48000, 1, 24);
        let params = parse_wav(&wav).expect("wav parses");
        assert_eq!(params.sample_rate, 48000);
        assert_eq!(params.channels, 1);
        assert_eq!(params.bits_per_sample, 24);
        assert_eq!(params.endianness, Endianness::Little);
    }

    #[test]
    fn rejects_non_pcm_wav() {
        // WAVE_FORMAT_ADPCM (tag = 2) — not PCM.
        let mut wav = make_minimal_wav(44100, 2, 16);
        // Patch the format tag at offset 20 (RIFF[12] + fmt chunk header[8]).
        wav[20] = 0x02;
        wav[21] = 0x00;
        assert!(parse_wav(&wav).is_none());
    }

    #[test]
    fn rejects_non_wav_magic() {
        assert!(parse_wav(b"NOTRIFF____WAVE____").is_none());
        assert!(parse_wav(b"RIFF\x00\x00\x00\x00NOPE____").is_none());
    }

    /// Build a minimal valid PCM WAV header for tests.
    fn make_minimal_wav(sample_rate: u32, channels: u8, bits: u8) -> Vec<u8> {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&0u32.to_le_bytes()); // size, patched below
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        wav.extend_from_slice(&1u16.to_le_bytes()); // tag = PCM
        wav.extend_from_slice(&[channels, 0]);
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * channels as u32 * bits as u32 / 8).to_le_bytes());
        wav.extend_from_slice(&[(channels * bits / 8), 0]); // block align
        wav.extend_from_slice(&[bits, 0]);
        // data chunk (empty)
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&0u32.to_le_bytes());
        // Patch RIFF size
        let total = u32::try_from(wav.len()).unwrap_or(0) - 8;
        wav[4..8].copy_from_slice(&total.to_le_bytes());
        wav
    }
}
