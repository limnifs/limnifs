//! `LimniFS` core reader: manifest header + section parsers, plus the
//! drop-store primitives (slab header, drop record, slab index).
//!
//! Source of truth: `limnifs/spec` §3 (drop store), §5 (manifest
//! sections), and the bit-level files under `bit-level/`.
//!
//! Architecture: every parser takes a [`cursor::ManifestCursor`] and
//! returns the parsed value plus the cursor advanced past the section.
//! Bounds checks live in one place (the cursor); each parser focuses
//! on the structural invariants of its own section. Adding a new
//! section is a new module + a new parser function — no edits to
//! existing parsers ([OCP](https://en.wikipedia.org/wiki/Open%E2%80%93closed_principle)).
//!
//! ## Module map
//!
//! | Module | Owns |
//! |---|---|
//! | [`cursor`] | [`ManifestCursor`] — bounded reader over `&[u8]` |
//! | [`error`] | [`CoreError`] |
//! | [`header`] | [`ManifestHeader`] + [`parse_manifest_header`] |
//! | [`feature_flags`] | [`FeatureFlag`], [`FeatureFlags`] + [`parse_feature_flags_section`] |
//! | [`metadata_reference`] | [`MetadataReference`] + [`parse_metadata_reference`] |
//! | [`slab_index`] | [`SlabIndex`], [`SlabIndexEntry`] + [`parse_slab_index`] |
//! | [`slab`] | [`SlabHeader`] + [`parse_slab_header`] |
//! | [`drop_record`] | [`DropRecord`] + [`parse_drop_record`] |
//! | [`locator`] | [`LocatorEntry`] + [`parse_locator_entry`] |

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

pub mod cursor;
pub mod drop_record;
pub mod error;
pub mod feature_flags;
pub mod header;
pub mod locator;
pub mod metadata_reference;
pub mod slab;
pub mod slab_index;

pub use cursor::ManifestCursor;
pub use drop_record::{
    parse_drop_record, parse_drop_record_with_ceiling, DropRecord, DROP_RECORD_LEN,
};
pub use error::CoreError;
pub use feature_flags::{
    parse_feature_flags_section, FeatureFlag, FeatureFlags, FEATURE_FLAGS_SECTION_VERSION,
};
pub use header::{parse_manifest_header, ManifestHeader};
pub use locator::{
    parse_locator_entries, parse_locator_entries_with_ceiling, parse_locator_entry,
    parse_locator_entry_with_ceiling, LocatorEntry, DEFAULT_LOCATOR_MAX_URI_BYTES,
    LOCATOR_LENGTH_PREFIX_LEN, MIN_LOCATOR_URI_BYTES,
};
pub use metadata_reference::{
    parse_metadata_reference, parse_metadata_reference_with_ceilings, MetadataReference,
    DEFAULT_INLINE_METADATA_MAX_BYTES, METADATA_REFERENCE_SECTION_VERSION,
};
pub use slab::{
    parse_slab_header, parse_slab_header_with_ceiling, SlabHeader, CRYPTO_HINT_EXTENDED,
    DEFAULT_SLAB_MAX_BYTES, EC_DESCRIPTOR_EXTENDED, SLAB_FORMAT_VERSION, SLAB_HEADER_LEN,
};
pub use slab_index::{
    parse_slab_index, parse_slab_index_with_ceiling, SlabIndex, SlabIndexEntry,
    SLAB_INDEX_SECTION_VERSION,
};

pub use limnifs_format::MANIFEST_HEADER_LEN;
