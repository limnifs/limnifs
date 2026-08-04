//! OCP registry for file-level categorizers.

use std::path::Path;

use super::{Categorization, FileCategorizer};

/// Process-wide registry of file-level categorizers.
///
/// Categorizers are consulted in registration order. The first one
/// to return `Some(Categorization)` wins; later ones are skipped.
///
/// Adding a categorizer:
/// 1. New file under `file_categorizer/<name>.rs`.
/// 2. Implement `FileCategorizer`.
/// 3. Push an instance into the registry builder.
///
/// Dispatch code (the `categorize` method) never changes when
/// adding/removing categorizers — that's the OCP win.
#[derive(Default)]
pub struct FileCategorizerRegistry {
    categorizers: Vec<Box<dyn FileCategorizer>>,
}

impl FileCategorizerRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a categorizer. Categorizers added later run later
    /// (lower priority). Register specific categorizers first.
    #[must_use]
    pub fn register(mut self, c: Box<dyn FileCategorizer>) -> Self {
        self.categorizers.push(c);
        self
    }

    /// Number of registered categorizers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.categorizers.len()
    }

    /// True iff no categorizers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.categorizers.is_empty()
    }

    /// Consult each categorizer in registration order. Returns the
    /// first non-`None` result, or `None` if no categorizer claims
    /// the file (caller should fall back to `FastCDC`).
    ///
    /// **Early-exit optimisation**: categorizers that declare a
    /// `first_byte_hint` are skipped without a function call when
    /// `data[0]` isn't in their hint set. This saves 3 out of 4
    /// categorizer calls on typical source files (only the CSV
    /// categorizer runs, since it has no hint).
    #[must_use]
    pub fn categorize(&self, path: &Path, data: &[u8]) -> Option<Categorization> {
        let first = data.first().copied();
        for c in &self.categorizers {
            // Early-exit: if the categorizer declares a first-byte
            // hint, check it before calling categorize().
            if let Some(hint) = c.first_byte_hint() {
                if let Some(byte) = first {
                    if !hint.contains(&byte) {
                        continue;
                    }
                }
            }
            if let Some(cat) = c.categorize(path, data) {
                return Some(cat);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct AlwaysClaim {
        name: &'static str,
        codec: u8,
    }

    impl FileCategorizer for AlwaysClaim {
        fn name(&self) -> &'static str {
            self.name
        }
        fn categories(&self) -> &'static [&'static str] {
            &["test"]
        }
        fn categorize(&self, _path: &Path, _data: &[u8]) -> Option<Categorization> {
            Some(Categorization {
                codec_id: self.codec,
                codec_params: Vec::new(),
                category: "test",
            })
        }
    }

    struct NeverClaim;

    impl FileCategorizer for NeverClaim {
        fn name(&self) -> &'static str {
            "never"
        }
        fn categories(&self) -> &'static [&'static str] {
            &[]
        }
        fn categorize(&self, _path: &Path, _data: &[u8]) -> Option<Categorization> {
            None
        }
    }

    #[test]
    fn empty_registry_returns_none() {
        let reg = FileCategorizerRegistry::new();
        assert!(reg.is_empty());
        assert!(reg.categorize(&PathBuf::from("/x"), b"abc").is_none());
    }

    #[test]
    fn first_match_wins() {
        let reg = FileCategorizerRegistry::new()
            .register(Box::new(AlwaysClaim {
                name: "first",
                codec: 0x10,
            }))
            .register(Box::new(AlwaysClaim {
                name: "second",
                codec: 0x20,
            }));
        let cat = reg
            .categorize(&PathBuf::from("/x"), b"abc")
            .expect("first claims");
        assert_eq!(cat.codec_id, 0x10);
    }

    #[test]
    fn falls_through_to_next_when_first_passes() {
        let reg = FileCategorizerRegistry::new()
            .register(Box::new(NeverClaim))
            .register(Box::new(AlwaysClaim {
                name: "second",
                codec: 0x20,
            }));
        let cat = reg
            .categorize(&PathBuf::from("/x"), b"abc")
            .expect("second claims");
        assert_eq!(cat.codec_id, 0x20);
    }
}
