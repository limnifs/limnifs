//! Fuzz target for `parse_locator_entry`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use limnifs_core::{ManifestCursor, parse_locator_entry};

fuzz_target!(|data: &[u8]| {
    let mut cursor = ManifestCursor::new(data);
    let _ = parse_locator_entry(&mut cursor);
});
