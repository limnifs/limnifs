//! Locator fetch trait — abstracts how slab bytes and metadata blobs
//! are fetched from storage at runtime.
//!
//! The wire-format [`crate::locator`] module parses locator URIs from
//! the manifest. This module provides the runtime abstraction for
//! actually fetching the bytes those URIs reference.
//!
//! ## URI scheme conventions
//!
//! | Scheme | Implementation | Notes |
//! |---|---|---|
//! | `file:` | [`FileLocator`] | Local path relative to manifest |
//! | `http:` / `https:` | [`crate::http_locator::HttpLocator`] | HTTP range streaming (08-locators) |
//! | `s3:` | _future_ | S3 GetObject (08-locators) |
//! | `ipfs:` | _future_ | IPFS CAR (Phase 3) |

use std::path::{Path, PathBuf};

/// Error from a [`Locator`] operation.
#[derive(Debug)]
pub enum LocatorError {
    /// The URI scheme is not recognised by this locator.
    UnsupportedScheme { scheme: String },
    /// The URI is malformed (missing path, bad encoding, etc.).
    InvalidUri { reason: String },
    /// The underlying storage returned an I/O error.
    Io(std::io::Error),
    /// The resource was not found.
    NotFound,
    /// The server returned an error status (HTTP-only).
    Status { code: u16, body: String },
}

impl std::fmt::Display for LocatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedScheme { scheme } => {
                write!(f, "unsupported locator scheme: {scheme}")
            }
            Self::InvalidUri { reason } => write!(f, "invalid URI: {reason}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::NotFound => write!(f, "resource not found"),
            Self::Status { code, body } => {
                write!(f, "HTTP {code}: {body}")
            }
        }
    }
}

impl std::error::Error for LocatorError {}

impl From<std::io::Error> for LocatorError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            Self::NotFound
        } else {
            Self::Io(e)
        }
    }
}

/// A storage backend that resolves locator URIs to bytes.
///
/// Implementations MUST be deterministic: the same URI always yields
/// the same bytes. This invariant is what makes content-addressed
/// dedup safe.
///
/// # OCP
///
/// New locator backends (HTTP, S3, IPFS) are added by implementing
/// this trait — no changes to existing code.
pub trait Locator: Send + Sync {
    /// Fetch the bytes at `uri`.
    ///
    /// # Errors
    ///
    /// Returns [`LocatorError`] if the URI is unsupported, malformed,
    /// or the fetch fails.
    fn fetch(&self, uri: &str) -> Result<Vec<u8>, LocatorError>;

    /// Fetch a byte range `[offset, offset + length)` from `uri`.
    ///
    /// The default implementation fetches the full resource and slices
    /// in memory; locators that support range requests natively (HTTP,
    /// S3) override this to avoid downloading the entire resource.
    ///
    /// `offset` past EOF returns an empty `Vec`. `offset + length` past
    /// EOF returns the available suffix (clamped to the resource size).
    ///
    /// # Errors
    ///
    /// Returns [`LocatorError`] if the URI is unsupported, malformed,
    /// or the fetch fails.
    fn fetch_range(&self, uri: &str, offset: u64, length: u64) -> Result<Vec<u8>, LocatorError> {
        let data = self.fetch(uri)?;
        let total = u64::try_from(data.len()).unwrap_or(u64::MAX);
        let start = offset.min(total);
        let end = start.saturating_add(length).min(total);
        let start_us = usize::try_from(start).unwrap_or(usize::MAX);
        let end_us = usize::try_from(end).unwrap_or(usize::MAX);
        Ok(data[start_us..end_us].to_vec())
    }

    /// The URI scheme this locator handles (e.g. `"file"`).
    fn scheme(&self) -> &'static str;
}

/// A locator that reads from the local filesystem.
#[derive(Debug, Clone)]
pub struct FileLocator {
    base_dir: PathBuf,
}

impl FileLocator {
    /// Create a new file locator rooted at `base_dir`.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Create a locator rooted at the parent of `manifest_path`.
    #[must_use]
    pub fn for_manifest(manifest_path: &Path) -> Self {
        let base_dir = manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Self { base_dir }
    }

    fn resolve(&self, uri: &str) -> Result<PathBuf, LocatorError> {
        let path_str =
            uri.strip_prefix("file:")
                .ok_or_else(|| LocatorError::UnsupportedScheme {
                    scheme: scheme_of(uri).to_owned(),
                })?;
        if path_str.is_empty() {
            return Err(LocatorError::InvalidUri {
                reason: "file: URI has empty path".into(),
            });
        }
        Ok(self.base_dir.join(path_str))
    }
}

impl Locator for FileLocator {
    fn fetch(&self, uri: &str) -> Result<Vec<u8>, LocatorError> {
        let path = self.resolve(uri)?;
        if !path.exists() {
            return Err(LocatorError::NotFound);
        }
        Ok(std::fs::read(&path)?)
    }

    fn scheme(&self) -> &'static str {
        "file"
    }
}

/// A dispatcher that routes URIs to the appropriate [`Locator`] by
/// scheme.
#[derive(Default)]
pub struct MultiLocator {
    locators: Vec<Box<dyn Locator>>,
}

impl MultiLocator {
    /// Create an empty dispatcher.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a locator (builder pattern).
    #[must_use]
    pub fn with(mut self, locator: Box<dyn Locator>) -> Self {
        self.locators.push(locator);
        self
    }

    /// Register a locator (chain pattern).
    pub fn register(&mut self, locator: Box<dyn Locator>) -> &mut Self {
        self.locators.push(locator);
        self
    }
}

impl Locator for MultiLocator {
    fn fetch(&self, uri: &str) -> Result<Vec<u8>, LocatorError> {
        let scheme = scheme_of(uri);
        for locator in &self.locators {
            if locator.scheme() == scheme {
                return locator.fetch(uri);
            }
        }
        Err(LocatorError::UnsupportedScheme {
            scheme: scheme.to_owned(),
        })
    }

    fn scheme(&self) -> &'static str {
        "multi"
    }
}

fn scheme_of(uri: &str) -> &str {
    match uri.find(':') {
        Some(pos) => &uri[..pos],
        None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn file_locator_reads_local_file() {
        let dir =
            std::env::temp_dir().join(format!("limnifs-locator-trait-{}-1", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("data.bin");
        let mut file = std::fs::File::create(&path).expect("create");
        file.write_all(b"hello locator").expect("write");
        let locator = FileLocator::new(&dir);
        let data = locator.fetch("file:data.bin").expect("fetch");
        assert_eq!(data, b"hello locator");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_locator_rejects_non_file_scheme() {
        let locator = FileLocator::new("/tmp");
        match locator.fetch("https://example.com/data.bin") {
            Err(LocatorError::UnsupportedScheme { scheme }) => {
                assert_eq!(scheme, "https");
            }
            other => panic!("expected UnsupportedScheme, got {other:?}"),
        }
    }

    #[test]
    fn file_locator_rejects_empty_path() {
        let locator = FileLocator::new("/tmp");
        match locator.fetch("file:") {
            Err(LocatorError::InvalidUri { .. }) => {}
            other => panic!("expected InvalidUri, got {other:?}"),
        }
    }

    #[test]
    fn file_locator_returns_not_found() {
        let locator = FileLocator::new("/tmp");
        match locator.fetch("file:nonexistent-12345.bin") {
            Err(LocatorError::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn multi_locator_dispatches_by_scheme() {
        let dir =
            std::env::temp_dir().join(format!("limnifs-locator-trait-{}-2", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("slab-0.bin");
        let mut file = std::fs::File::create(&path).expect("create");
        file.write_all(b"slab data").expect("write");

        let multi = MultiLocator::new().with(Box::new(FileLocator::new(&dir)));
        let data = multi.fetch("file:slab-0.bin").expect("fetch");
        assert_eq!(data, b"slab data");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn multi_locator_rejects_unregistered_scheme() {
        let multi = MultiLocator::new();
        match multi.fetch("s3://bucket/key") {
            Err(LocatorError::UnsupportedScheme { scheme }) => {
                assert_eq!(scheme, "s3");
            }
            other => panic!("expected UnsupportedScheme, got {other:?}"),
        }
    }

    #[test]
    fn scheme_of_extracts_prefix() {
        assert_eq!(scheme_of("file:data.bin"), "file");
        assert_eq!(scheme_of("https://example.com/x"), "https");
        assert_eq!(scheme_of("s3://bucket/key"), "s3");
        assert_eq!(scheme_of("no-scheme"), "");
    }

    #[test]
    fn for_manifest_uses_parent_dir() {
        let path = std::path::Path::new("/tmp/manifest.lim");
        let locator = FileLocator::for_manifest(path);
        assert_eq!(locator.base_dir, std::path::Path::new("/tmp"));
    }
}
