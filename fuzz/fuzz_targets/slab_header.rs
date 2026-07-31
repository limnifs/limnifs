//! Fuzz target for `parse_slab_header`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use limnifs_core::{ManifestCursor, parse_slab_header};

fuzz_target!(|data: &[u8]| {
    let mut cursor = ManifestCursor::new(data);
    let _ = parse_slab_header(&mut cursor);
});
