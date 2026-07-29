//! `LimniFS` core reader: manifest header + section parsers.
//!
//! Source of truth: `limnifs/spec` §5 (manifest sections) and the
//! bit-level files under `bit-level/` (35-manifest-header.md,
//! 36-feature-flags.md, …).
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

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

pub mod cursor;
pub mod error;
pub mod feature_flags;
pub mod header;

pub use cursor::ManifestCursor;
pub use error::CoreError;
pub use feature_flags::{
    parse_feature_flags_section, FeatureFlag, FeatureFlags, FEATURE_FLAGS_SECTION_VERSION,
};
pub use header::{parse_manifest_header, ManifestHeader};

pub use limnifs_format::MANIFEST_HEADER_LEN;
