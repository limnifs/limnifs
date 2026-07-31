//! Fuzz target for `parse_metadata_blob`.
//!
//! This one is critical: the metadata blob has count prefixes that
//! could cause huge allocations. The parser has `DoS` guards (see
//! `parse_metadata_blob_with_ceiling`); the fuzzer verifies no
//! input — however adversarial — causes a panic or runaway alloc.

#![no_main]

use libfuzzer_sys::fuzz_target;
use limnifs_core::{ManifestCursor, parse_metadata_blob};

fuzz_target!(|data: &[u8]| {
    let mut cursor = ManifestCursor::new(data);
    let _ = parse_metadata_blob(&mut cursor);
});
