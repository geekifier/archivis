use std::path::PathBuf;

use archivis_core::errors::{DbError, FormatError, StorageError};
use archivis_core::models::ScoringProfile;
use archivis_formats::sanitize::SanitizeOptions;
use uuid::Uuid;

/// Configuration for the import service.
///
/// Boot-frozen fields only — runtime knobs (e.g. `auto_link_formats`) are
/// read at point-of-use from `SettingsReader`.
#[derive(Debug)]
pub struct ImportConfig {
    /// Directory for cache data (covers, thumbnails).
    pub data_dir: PathBuf,
    /// Thumbnail size targets.
    pub thumbnail_sizes: ThumbnailSizes,
    /// Options for sanitizing metadata text fields during import.
    pub sanitize_options: SanitizeOptions,
    /// Scoring profile for metadata quality evaluation.
    pub scoring_profile: ScoringProfile,
}

impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from(".local"),
            thumbnail_sizes: ThumbnailSizes::default(),
            sanitize_options: SanitizeOptions::default(),
            scoring_profile: ScoringProfile::default(),
        }
    }
}

/// Target heights for generated thumbnails (width preserves aspect ratio).
#[derive(Debug)]
pub struct ThumbnailSizes {
    /// Small thumbnail height in pixels.
    pub sm_height: u32,
    /// Medium thumbnail height in pixels.
    pub md_height: u32,
}

impl Default for ThumbnailSizes {
    fn default() -> Self {
        Self {
            sm_height: 150,
            md_height: 300,
        }
    }
}

/// Result of a successful single-file import.
#[derive(Debug)]
pub struct ImportResult {
    pub book_id: Uuid,
    pub book_file_id: Uuid,
    pub status: archivis_core::models::MetadataStatus,
    pub confidence: f32,
    pub duplicate: Option<DuplicateInfo>,
    pub cover_extracted: bool,
}

/// Information about a duplicate detected during import.
#[derive(Debug)]
pub enum DuplicateInfo {
    /// Exact same file (by SHA-256 hash) already exists.
    ExactHash { existing_book_id: Uuid },
    /// A different file with the same ISBN already exists for the same format.
    SameIsbn {
        existing_book_id: Uuid,
        isbn: String,
    },
    /// Fuzzy title+author match — soft duplicate that does not block import.
    FuzzyMatch {
        existing_book_id: Uuid,
        title_similarity: f32,
        author_similarity: f32,
    },
}

/// Errors that can occur during a single-file import.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("format detection failed: {0}")]
    FormatDetection(#[from] FormatError),

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("database error: {0}")]
    Database(#[from] DbError),

    #[error("duplicate file: hash {hash} already belongs to book {existing_book_id}")]
    DuplicateFile {
        existing_book_id: Uuid,
        hash: String,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid file: {0}")]
    InvalidFile(String),
}
