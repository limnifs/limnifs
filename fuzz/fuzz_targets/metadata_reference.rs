//! Fuzz target for `parse_metadata_reference`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use limnifs_core::{ManifestCursor, parse_metadata_reference};

fuzz_target!(|data: &[u8]| {
    let mut cursor = ManifestCursor::new(data);
    let _ = parse_metadata_reference(&mut cursor);
});
