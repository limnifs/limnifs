//! File-level categorizer framework.
//!
//! The seine chunk classifier (`crate::classifier`) operates on
//! chunks AFTER FastCDC has split a file. By that point file-level
//! signal is gone — a FITS header lives in chunk 0; chunk 50 looks
//! like generic binary. Specialized codecs (FLAC for PCM audio,
//! ricepp for FITS) need that file-level signal to route correctly.
//!
//! This module runs file-level categorizers BEFORE FastCDC. If a
//! categorizer claims the file, the whole file becomes one drop
//! compressed with the categorizer's chosen codec. Otherwise the
//! file falls through to the existing FastCDC path unchanged.
//!
//! ## Architecture
//!
//! - [`FileCategorizer`] trait: synchronous, pure-functional,
//!   deterministic. Same `(path, data)` → same `Categorization`.
//! - [`FileCategorizerRegistry`]: OCP. Adding a categorizer is one
//!   new file + one `register()` call. Dispatch code never changes.
//! - The registry is consulted by `process_file` before FastCDC.
//!
//! ## Current state
//!
//! The registry ships EMPTY today. Categorizers for FLAC (PCM audio),
//! ricepp (FITS), and FSST (CSV/JSON) will be added when the
//! corresponding omnizip codec crates ship. The framework is in
//! place so the integration is a one-file PR per codec.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::path::Path;
use std::sync::OnceLock;

pub mod pcm_audio;
pub mod fits;
pub mod csv_text;
pub mod registry;

pub use registry::FileCategorizerRegistry;

/// Process-wide default registry. Populated on first access with
/// every shipped categorizer (pcm_audio, fits, csv_text). The
/// writer calls `default_registry().categorize(...)` from
/// `process_file` before FastCDC; categorizers that aren't
/// enabled (their `*_ENABLED` flag is false) return `None`
/// internally and fall through.
///
/// Adding a categorizer: implement `FileCategorizer`, push an
/// instance here. Dispatch code never changes.
#[must_use]
pub fn default_registry() -> &'static FileCategorizerRegistry {
    static REGISTRY: OnceLock<FileCategorizerRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        FileCategorizerRegistry::new()
            .register(Box::new(fits::FitsCategorizer))
            .register(Box::new(pcm_audio::PcmAudioCategorizer))
            .register(Box::new(csv_text::CsvTextCategorizer))
    })
}

/// A categorizer's decision for one file.
///
/// `codec_id` selects the codec; `codec_params` carries any
/// codec-specific parameters the categorizer extracted from the
/// file header (e.g. PCM sample format for FLAC, bitpix for
/// ricepp). The codec crate owns its parameter format; the
/// categorizer just hands opaque bytes through.
#[derive(Clone, Debug)]
pub struct Categorization {
    /// Codec id from `limnifs_core::codec` (e.g. `CODEC_FLAC`).
    pub codec_id: u8,
    /// Codec-specific parameters extracted from the file header.
    /// Encoded format is owned by the codec crate; opaque to the
    /// framework.
    pub codec_params: Vec<u8>,
    /// Human-readable category name for diagnostics
    /// (e.g. `"pcmaudio/waveform"`, `"fits/image"`).
    pub category: &'static str,
}

/// One file-level categorizer.
///
/// Implementations should be:
/// - **Pure-functional**: same input → same output, no I/O.
/// - **Deterministic**: no clocks, no RNG, no system state.
/// - **Cheap to refuse**: header parsing should bail on the first
///   mismatched magic byte, not scan the whole file.
///
/// Categorizers are tried in registration order. The first one to
/// return `Some(Categorization)` wins; later categorizers are not
/// consulted. Order matters: register specific categorizers before
/// generic ones.
pub trait FileCategorizer: Sync + Send {
    /// Unique name for logging/diagnostics.
    fn name(&self) -> &'static str;

    /// Categories this categorizer can emit. Used for diagnostic
    /// dumps; does not affect dispatch.
    fn categories(&self) -> &'static [&'static str];

    /// Categorize a file by its path and full contents.
    ///
    /// Returns `Some(Categorization)` if this categorizer claims the
    /// file, `None` to defer to the next categorizer in the registry
    /// (or to the FastCDC fallback path).
    ///
    /// Implementations should not read the file from disk — `data`
    /// is already in hand. Path is provided for extension-based
    /// hints when magic-byte detection is ambiguous.
    fn categorize(&self, path: &Path, data: &[u8]) -> Option<Categorization>;
}
