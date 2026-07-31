//! Fuzz target for `parse_history`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use limnifs_core::{ManifestCursor, parse_history};

fuzz_target!(|data: &[u8]| {
    let mut cursor = ManifestCursor::new(data);
    let _ = parse_history(&mut cursor);
});
