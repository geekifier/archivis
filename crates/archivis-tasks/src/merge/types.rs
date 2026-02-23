use std::fmt;

use archivis_core::errors::{DbError, StorageError};
use uuid::Uuid;

/// Which book's metadata to prefer when fields conflict during merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergePreference {
    /// Keep primary's value unless it's NULL/empty, then take secondary's.
    Primary,
    /// Use secondary's value if present, else keep primary's.
    Secondary,
    /// Use whichever book has higher `metadata_confidence`.
    HigherConfidence,
}

impl MergePreference {
    /// Parse from a string, defaulting to `Primary` for unrecognised values.
    pub fn from_str_or_default(s: Option<&str>) -> Self {
        match s {
            Some("secondary") => Self::Secondary,
            Some("higher_confidence") => Self::HigherConfidence,
            _ => Self::Primary,
        }
    }
}

impl fmt::Display for MergePreference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primary => write!(f, "primary"),
            Self::Secondary => write!(f, "secondary"),
            Self::HigherConfidence => write!(f, "higher_confidence"),
        }
    }
}

/// Options controlling the merge behaviour.
#[derive(Debug, Clone)]
pub struct MergeOptions {
    /// Which book's metadata to prefer for conflicting fields.
    pub prefer_metadata_from: MergePreference,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            prefer_metadata_from: MergePreference::Primary,
        }
    }
}

/// Errors that can occur during a book merge operation.
#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    #[error("book not found: {0}")]
    BookNotFound(Uuid),

    #[error("cannot merge a book with itself")]
    SameBook,

    #[error("database error: {0}")]
    Database(#[from] DbError),

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_preference_from_str() {
        assert_eq!(
            MergePreference::from_str_or_default(None),
            MergePreference::Primary,
        );
        assert_eq!(
            MergePreference::from_str_or_default(Some("primary")),
            MergePreference::Primary,
        );
        assert_eq!(
            MergePreference::from_str_or_default(Some("secondary")),
            MergePreference::Secondary,
        );
        assert_eq!(
            MergePreference::from_str_or_default(Some("higher_confidence")),
            MergePreference::HigherConfidence,
        );
        assert_eq!(
            MergePreference::from_str_or_default(Some("unknown")),
            MergePreference::Primary,
        );
    }

    #[test]
    fn merge_preference_display() {
        assert_eq!(MergePreference::Primary.to_string(), "primary");
        assert_eq!(MergePreference::Secondary.to_string(), "secondary");
        assert_eq!(
            MergePreference::HigherConfidence.to_string(),
            "higher_confidence"
        );
    }

    #[test]
    fn merge_options_default_is_primary() {
        let opts = MergeOptions::default();
        assert_eq!(opts.prefer_metadata_from, MergePreference::Primary);
    }

    #[test]
    fn merge_error_display() {
        let err = MergeError::SameBook;
        assert_eq!(err.to_string(), "cannot merge a book with itself");

        let id = Uuid::new_v4();
        let err = MergeError::BookNotFound(id);
        assert_eq!(err.to_string(), format!("book not found: {id}"));
    }
}
