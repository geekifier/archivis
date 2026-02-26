use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::enums::BookFormat;

/// A physical file associated with a book. A single book can have multiple files
/// in different formats (e.g., EPUB + PDF of the same title).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookFile {
    pub id: Uuid,
    pub book_id: Uuid,
    pub format: BookFormat,
    /// Format specification version (e.g., "3.0" for EPUB 3.0, "1.7" for PDF 1.7).
    pub format_version: Option<String>,
    /// Relative path within the storage backend.
    pub storage_path: String,
    /// File size in bytes.
    pub file_size: i64,
    /// SHA-256 hash of the file contents, hex-encoded.
    pub hash: String,
    pub added_at: DateTime<Utc>,
}

impl BookFile {
    pub fn new(
        book_id: Uuid,
        format: BookFormat,
        storage_path: impl Into<String>,
        file_size: i64,
        hash: impl Into<String>,
        format_version: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            book_id,
            format,
            format_version,
            storage_path: storage_path.into(),
            file_size,
            hash: hash.into(),
            added_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_book_file() {
        let book_id = Uuid::new_v4();
        let file = BookFile::new(
            book_id,
            BookFormat::Epub,
            "H/Herbert, Frank/Dune/dune.epub",
            1_048_576,
            "abc123def456",
            Some("3.0".into()),
        );
        assert_eq!(file.book_id, book_id);
        assert_eq!(file.format, BookFormat::Epub);
        assert_eq!(file.file_size, 1_048_576);
    }

    #[test]
    fn book_file_serde_roundtrip() {
        let file = BookFile::new(
            Uuid::new_v4(),
            BookFormat::Pdf,
            "path/to/book.pdf",
            512_000,
            "deadbeef",
            None,
        );
        let json = serde_json::to_string(&file).unwrap();
        let deserialized: BookFile = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, file);
    }
}
