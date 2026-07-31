//! Fuzz target for `parse_drop_record`.
//!
//! Requires a slab header prefix; the fuzzer splits its input into
//! header-portion + drop-record-portion to provide both contexts.

#![no_main]

use libfuzzer_sys::fuzz_target;
use limnifs_core::{ManifestCursor, parse_drop_record, parse_slab_header};

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }
    let split_at = usize::from(data[0]).min(data.len() - 1);
    let (header_bytes, drop_bytes) = data[1..].split_at(split_at);

    let mut h_cursor = ManifestCursor::new(header_bytes);
    if let Ok(header) = parse_slab_header(&mut h_cursor) {
        let mut d_cursor = ManifestCursor::new(drop_bytes);
        let _ = parse_drop_record(&mut d_cursor, &header);
    }
});
