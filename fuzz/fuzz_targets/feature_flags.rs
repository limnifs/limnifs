//! Fuzz target for `parse_feature_flags_section`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use limnifs_core::{ManifestCursor, parse_feature_flags_section};

fuzz_target!(|data: &[u8]| {
    let mut cursor = ManifestCursor::new(data);
    let _ = parse_feature_flags_section(&mut cursor);
});
