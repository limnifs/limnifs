//! File-level categorizer framework.
//!
//! The seine chunk classifier (`crate::classifier`) operates on
//! chunks AFTER `FastCDC` has split a file. By that point file-level
//! signal is gone — a FITS header lives in chunk 0; chunk 50 looks
//! like generic binary. Specialized codecs (FLAC for PCM audio,
//! ricepp for FITS) need that file-level signal to route correctly.
//!
//! This module runs file-level categorizers BEFORE `FastCDC`. If a
//! categorizer claims the file, the whole file becomes one drop
//! compressed with the categorizer's chosen codec. Otherwise the
//! file falls through to the existing `FastCDC` path unchanged.
//!
//! ## Architecture
//!
//! - [`FileCategorizer`] trait: synchronous, pure-functional,
//!   deterministic. Same `(path, data)` → same `Categorization`.
//! - [`FileCategorizerRegistry`]: OCP. Adding a categorizer is one
//!   new file + one `register()` call. Dispatch code never changes.
//! - The registry is consulted by `process_file` before `FastCDC`.
//!
//! ## Current state
//!
//! The registry ships EMPTY today. Categorizers for FLAC (PCM audio),
//! ricepp (FITS), and FSST (CSV/JSON) will be added when the
//! corresponding omnizip codec crates ship. The framework is in
//! place so the integration is a one-file PR per codec.

use std::path::Path;
use std::sync::OnceLock;

pub mod csv_text;
pub mod executable;
pub mod fits;
pub mod pcm_audio;
pub mod registry;

pub use registry::FileCategorizerRegistry;

/// Process-wide default registry. Populated on first access with
/// every shipped categorizer (`pcm_audio`, fits, `csv_text`). The
/// writer calls `default_registry().categorize(...)` from
/// `process_file` before `FastCDC`; categorizers that aren't
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
            .register(Box::new(executable::ExecutableCategorizer))
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
    /// (or to the `FastCDC` fallback path).
    ///
    /// Implementations should not read the file from disk — `data`
    /// is already in hand. Path is provided for extension-based
    /// hints when magic-byte detection is ambiguous.
    fn categorize(&self, path: &Path, data: &[u8]) -> Option<Categorization>;

    /// The set of first bytes that this categorizer can possibly
    /// match. The registry uses this to skip categorizers without
    /// a function call when `data[0]` isn't in the set.
    ///
    /// Return `None` (default) to opt out of the early-exit
    /// optimisation — the categorizer is always tried. Return
    /// `Some(&[bytes])` to enable early-exit: the registry checks
    /// `data[0]` and skips this categorizer if it's not in the set.
    ///
    /// Example: ELF categorizer returns `Some(&[0x7F])` — it can
    /// only match files whose first byte is 0x7F.
    fn first_byte_hint(&self) -> Option<&'static [u8]> {
        None
    }
}

/// A `FileCategorizer` built from a slice of `CategorizerConfig`
/// entries (the user-facing TOML surface). Each entry is matched
/// by extension (lowercased file suffix) or by the leading
/// `magic_bytes`; the first match wins. Used alongside the static
/// built-in registry so users can route `.bin` / `.dat` /
/// extensionless executables without recompiling. Fixes
/// `limnifs#196`.
#[derive(Debug)]
pub struct ConfigCategorizer {
    entries: Vec<super::config::CategorizerConfig>,
}

impl ConfigCategorizer {
    #[must_use]
    pub fn new(entries: Vec<super::config::CategorizerConfig>) -> Self {
        Self { entries }
    }
}

impl FileCategorizer for ConfigCategorizer {
    fn name(&self) -> &'static str {
        "config"
    }
    fn categories(&self) -> &'static [&'static str] {
        &["config"]
    }
    fn categorize(&self, path: &Path, data: &[u8]) -> Option<Categorization> {
        use super::config::CategorizerConfig;
        // Extension match (lowercased suffix, no leading dot).
        let ext_lower: Option<String> = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase());
        for entry in &self.entries {
            if !entry.enabled {
                continue;
            }
            let by_ext = ext_lower
                .as_deref()
                .is_some_and(|e| entry.extensions.iter().any(|x| x == e));
            let by_magic = !entry.magic_bytes.is_empty() && data.starts_with(&entry.magic_bytes);
            if !by_ext && !by_magic {
                continue;
            }
            if let Some(max) = entry.max_size {
                if u64::try_from(data.len()).unwrap_or(u64::MAX) > u64::from(max) {
                    continue;
                }
            }
            // Resolve the codec by name from the writer's registry.
            // (The caller — process_file — does the resolution; here
            // we tag a flag and the writer handles the lookup via
            // a separate helper. See `resolve_codec`.)
            return Some(Categorization {
                codec_id: 0, // sentinel; resolved by caller
                codec_params: encode_config_ref(entry),
                category: "config",
            });
        }
        None
    }
}

fn encode_config_ref(entry: &super::config::CategorizerConfig) -> Vec<u8> {
    // Small length-prefixed encoding: u32 name_len, name bytes,
    // u8 enabled. The caller uses the entry's `name` to look up
    // the codec — the Vec is just a handle to identify the
    // matched CategorizerConfig.
    let mut out = Vec::with_capacity(4 + entry.name.len() + 1);
    let len = u32::try_from(entry.name.len()).unwrap_or(0);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(entry.name.as_bytes());
    out.push(u8::from(entry.enabled));
    out
}

/// Resolve a `Categorization::category == "config"` back to the
/// original `CategorizerConfig` and its codec id via the writer's
/// `WriteConfig::codec_registry`. Returns the codec id and the
/// raw `codec_params` from the config entry (not the encoded
/// handle).
pub fn resolve_config_categorization(
    cat: &Categorization,
    entries: &[super::config::CategorizerConfig],
    codec_resolver: &dyn Fn(&str) -> Option<u8>,
) -> Option<u8> {
    if cat.category != "config" {
        return None;
    }
    if cat.codec_params.len() < 5 {
        return None;
    }
    let name_len = u32::from_le_bytes(cat.codec_params[..4].try_into().ok()?) as usize;
    let rest = &cat.codec_params[4..];
    if rest.len() < name_len + 1 {
        return None;
    }
    let name = std::str::from_utf8(&rest[..name_len]).ok()?;
    let entry = entries.iter().find(|c| c.name == name)?;
    if !entry.enabled {
        return None;
    }
    codec_resolver(&entry.codec)
}
