//! Fuzz target for `parse_manifest_header`.
//!
//! The header parser must accept any byte sequence: either return a
//! valid header or return an `Err`. It must NEVER panic.

#![no_main]

use libfuzzer_sys::fuzz_target;
use limnifs_core::{ManifestCursor, parse_manifest_header};

fuzz_target!(|data: &[u8]| {
    let mut cursor = ManifestCursor::new(data);
    let _ = parse_manifest_header(&mut cursor);
});
