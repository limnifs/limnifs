//! Errors returned by the limnifs-core parsers.
//!
//! All errors surface the kind of structural problem and enough detail
//! to produce a precise user-facing message. The `limni` CLI maps
//! these to stable exit codes; other consumers (e.g. mount layer,
//! adapters) can match on the variant for policy decisions.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use core::fmt;

/// Error reading a manifest header or section.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CoreError {
    /// Fewer than the required bytes available.
    TooShort { have: usize, need: usize },
    /// Magic bytes did not match the expected constant.
    BadMagic { found: [u8; 4] },
    /// A structural invariant was violated (nonzero reserved, bad
    /// section version, duplicate flag id, out-of-range value, etc.).
    Corrupt { reason: String },
    /// The image uses a feature the reader does not implement.
    /// `feature` carries enough context for the caller to report or
    /// match (flag id, section version, etc.).
    UnsupportedFeature { feature: String },
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { have, need } => {
                write!(
                    f,
                    "manifest header truncated: have {have} bytes, need {need}"
                )
            }
            Self::BadMagic { found } => write!(
                f,
                "bad manifest magic: expected LMFS ({:x?}), found {:?} ({:x?})",
                *b"LMFS",
                core::str::from_utf8(found).unwrap_or("<non-utf8>"),
                found
            ),
            Self::Corrupt { reason } => write!(f, "manifest corrupt: {reason}"),
            Self::UnsupportedFeature { feature } => {
                write!(f, "unsupported feature: {feature}")
            }
        }
    }
}

impl std::error::Error for CoreError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_covers_every_variant() {
        let cases = [
            (
                CoreError::TooShort { have: 4, need: 16 },
                vec!["truncated", "16"],
            ),
            (
                CoreError::BadMagic { found: *b"XXXX" },
                vec!["LMFS", "XXXX"],
            ),
            (
                CoreError::Corrupt {
                    reason: "broken".into(),
                },
                vec!["corrupt", "broken"],
            ),
            (
                CoreError::UnsupportedFeature {
                    feature: "feature_flags section version 7".into(),
                },
                vec!["unsupported", "version 7"],
            ),
        ];
        for (error, needles) in cases {
            let s = error.to_string();
            for needle in needles {
                assert!(s.contains(needle), "display {s:?} missing {needle:?}");
            }
        }
    }

    #[test]
    fn variants_are_eq_comparable() {
        assert_eq!(
            CoreError::TooShort { have: 4, need: 5 },
            CoreError::TooShort { have: 4, need: 5 }
        );
        assert_ne!(
            CoreError::TooShort { have: 4, need: 5 },
            CoreError::TooShort { have: 4, need: 6 }
        );
    }
}
