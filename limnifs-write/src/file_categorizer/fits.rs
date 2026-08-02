//! FITS image categorizer — routes FITS files to ricepp.
//!
//! **Status:** DETECTION READY, ROUTING DISABLED.
//!
//! FITS (Flexible Image Transport System) is the standard format
//! for astronomical images. Each file begins with a 2880-byte
//! header containing key-value records in ASCII. The key fields for
//! ricepp routing:
//!
//! - `SIMPLE  = T` — marks a primary header (always the first record)
//! - `BITPIX  = N` — bits per pixel (8, 16, 32, -32 for float, -64 for double)
//! - `NAXIS   = N` — number of axes (2 for a 2D image)
//! - `NAXISn  = N` — size of axis n
//!
//! All FITS files are big-endian.
//!
//! Routing is disabled until `omnizip-ricepp` ships a real Rice++
//! encoder. When it does, flip `RICEPP_ENABLED` to `true` and the
//! categorizer will claim FITS files for the ricepp codec (id 0x08).

use std::path::Path;

use super::{Categorization, FileCategorizer};
use limnifs_core::codec::CODEC_RICEPP;

/// Flip to `false` if ricepp routing causes regressions. Currently
/// always on because `omnizip-ricepp` 0.4 ships a working encoder
/// and the wrapper at `limnifs-core::codec::ricepp` round-trips.
const RICEPP_ENABLED: bool = true;

/// First 9 bytes of every FITS primary header.
const FITS_MAGIC: &[u8] = b"SIMPLE  =";

/// FITS records are 80 bytes, header blocks are 2880 bytes.
const FITS_RECORD_LEN: usize = 80;

/// Parameters extracted from the FITS header for the ricepp codec.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FitsParams {
    /// Bits per pixel (8, 16, 32, -32, -64).
    pub bitpix: i32,
    /// Number of axes (typically 2 for a 2D image).
    pub naxis: u32,
    /// Size of axis 1.
    pub naxis1: u32,
    /// Size of axis 2.
    pub naxis2: u32,
}

impl FitsParams {
    /// Encode as a compact 16-byte prefix the ricepp codec can decode.
    #[must_use]
    pub fn encode(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&self.bitpix.to_le_bytes());
        out[4..8].copy_from_slice(&self.naxis.to_le_bytes());
        out[8..12].copy_from_slice(&self.naxis1.to_le_bytes());
        out[12..16].copy_from_slice(&self.naxis2.to_le_bytes());
        out
    }
}

pub struct FitsCategorizer;

impl FileCategorizer for FitsCategorizer {
    fn name(&self) -> &'static str {
        "fits"
    }

    fn categories(&self) -> &'static [&'static str] {
        &["fits/image"]
    }

    fn categorize(&self, _path: &Path, data: &[u8]) -> Option<Categorization> {
        if !RICEPP_ENABLED {
            return None;
        }
        let params = parse_fits(data)?;
        Some(Categorization {
            codec_id: CODEC_RICEPP,
            codec_params: params.encode().to_vec(),
            category: "fits/image",
        })
    }
}

/// Parse the FITS primary header. Returns `None` if the magic is
/// missing or the required keys (`BITPIX`, `NAXIS`) can't be parsed.
#[must_use]
fn parse_fits(data: &[u8]) -> Option<FitsParams> {
    if data.len() < FITS_MAGIC.len() + FITS_RECORD_LEN {
        return None;
    }
    if &data[0..FITS_MAGIC.len()] != FITS_MAGIC {
        return None;
    }
    let mut bitpix: Option<i32> = None;
    let mut naxis: Option<u32> = None;
    let mut naxis1: Option<u32> = None;
    let mut naxis2: Option<u32> = None;

    // Walk up to the END record or the first header block boundary.
    let header_block = data.len().min(2880);
    let mut off = 0;
    while off + FITS_RECORD_LEN <= header_block {
        let record = &data[off..off + FITS_RECORD_LEN];
        let key = &record[0..8];
        let value = &record[10..30];
        if key.starts_with(b"END     ") {
            break;
        }
        match key {
            b"BITPIX  " => bitpix = parse_int(value),
            b"NAXIS   " => naxis = parse_int(value).map(|i| i as u32),
            b"NAXIS1  " => naxis1 = parse_int(value).map(|i| i as u32),
            b"NAXIS2  " => naxis2 = parse_int(value).map(|i| i as u32),
            _ => {}
        }
        off += FITS_RECORD_LEN;
    }
    Some(FitsParams {
        bitpix: bitpix?,
        naxis: naxis?,
        naxis1: naxis1.unwrap_or(0),
        naxis2: naxis2.unwrap_or(0),
    })
}

/// Parse a FORTRAN-style integer from a value field. Returns `None`
/// on parse failure.
fn parse_int(value: &[u8]) -> Option<i32> {
    let s = std::str::from_utf8(value).ok()?;
    s.trim().trim_end_matches('/').trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fits_record(key: &str, value: &str) -> [u8; FITS_RECORD_LEN] {
        let mut rec = [b' '; FITS_RECORD_LEN];
        let key_bytes = key.as_bytes();
        let copy_len = key_bytes.len().min(8);
        rec[..copy_len].copy_from_slice(&key_bytes[..copy_len]);
        rec[8] = b'=';
        rec[9] = b' ';
        let value_bytes = value.as_bytes();
        let copy_len = value_bytes.len().min(20);
        rec[10..10 + copy_len].copy_from_slice(&value_bytes[..copy_len]);
        rec
    }

    fn make_fits_header() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&make_fits_record("SIMPLE", "T"));
        buf.extend_from_slice(&make_fits_record("BITPIX", "16"));
        buf.extend_from_slice(&make_fits_record("NAXIS", "2"));
        buf.extend_from_slice(&make_fits_record("NAXIS1", "512"));
        buf.extend_from_slice(&make_fits_record("NAXIS2", "512"));
        buf.extend_from_slice(&make_fits_record("END", ""));
        // Pad to 2880.
        while buf.len() < 2880 {
            buf.push(0);
        }
        buf
    }

    #[test]
    fn fits_routes_to_ricepp_when_enabled() {
        let c = FitsCategorizer;
        let fits = make_fits_header();
        let cat = c
            .categorize(Path::new("/x.fits"), &fits)
            .expect("fits claims");
        assert_eq!(cat.codec_id, limnifs_core::codec::CODEC_RICEPP);
    }

    #[test]
    fn fits_header_parsed_correctly() {
        let fits = make_fits_header();
        let params = parse_fits(&fits).expect("fits parses");
        assert_eq!(params.bitpix, 16);
        assert_eq!(params.naxis, 2);
        assert_eq!(params.naxis1, 512);
        assert_eq!(params.naxis2, 512);
    }

    #[test]
    fn rejects_non_fits_magic() {
        let junk = vec![0u8; 4096];
        assert!(parse_fits(&junk).is_none());
    }
}
