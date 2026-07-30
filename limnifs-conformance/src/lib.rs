//! `LimniFS` conformance suite.
//!
//! Declarative test vectors plus a [`builder`] that encodes them into
//! valid `.lim` byte sequences. Each vector names its expected
//! [`limnifs_format::ManifestRoot`] so a pass means identity-level
//! agreement, not just "didn't crash".
//!
//! ## Layout
//!
//! | Module | Owns |
//! |---|---|
//! | [`builder`] | Manifest encoder: declarative spec → wire bytes + `ManifestRoot` |
//! | [`vectors`] | The declarative vectors themselves |
//! | [`harness`] | Round-trip harness: encode → parse → assert identity |
//!
//! ## Black-box invariant
//!
//! The conformance harness never links the reader-under-test as a
//! library — it speaks the format only. The [`builder`] DOES depend
//! on `limnifs-core` (it uses `compute_merkle_root` to derive expected
//! outputs), but that is the generator side, not the verification side.
//! Future third-party adapters plug in by reading the same fixture
//! bytes; they never share code with the generator.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

pub mod builder;
pub mod differential;
pub mod harness;
pub mod vectors;

pub use builder::{ManifestArtifact, ManifestBuilder, ManifestSpec};
pub use differential::{
    differential_rejection, differential_root, run_limni_py, run_limni_rust, should_run, CliReport,
    Mutation, DIFFERENTIAL_ENV_VAR,
};
pub use vectors::minimal_v0_1;
