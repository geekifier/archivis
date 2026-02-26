use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tracks a user's reading position within a specific book file.
/// Each combination of (user, `book_file`, device) has at most one progress record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingProgress {
    pub id: Uuid,
    pub user_id: Uuid,
    pub book_id: Uuid,
    pub book_file_id: Uuid,
    /// Format-specific location (e.g. EPUB CFI, PDF page number).
    pub location: Option<String>,
    /// Fraction of the book read (0.0 to 1.0).
    pub progress: f64,
    /// Device identifier. NULL = web browser.
    pub device_id: Option<String>,
    /// Reader preferences stored as JSON (font size, theme, etc.).
    pub preferences: Option<serde_json::Value>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A user-created bookmark within a book file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: Uuid,
    pub user_id: Uuid,
    pub book_id: Uuid,
    pub book_file_id: Uuid,
    /// Format-specific location (e.g. EPUB CFI).
    pub location: String,
    /// User-provided label for this bookmark.
    pub label: Option<String>,
    /// Text excerpt near the bookmark location.
    pub excerpt: Option<String>,
    /// Position within the book as a fraction (0.0 to 1.0).
    pub position: f64,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_progress_serde_roundtrip() {
        let progress = ReadingProgress {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            book_id: Uuid::new_v4(),
            book_file_id: Uuid::new_v4(),
            location: Some("epubcfi(/6/4)".into()),
            progress: 0.42,
            device_id: None,
            preferences: Some(serde_json::json!({"fontSize": 16})),
            started_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&progress).unwrap();
        let deserialized: ReadingProgress = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, progress);
    }

    #[test]
    fn bookmark_serde_roundtrip() {
        let bookmark = Bookmark {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            book_id: Uuid::new_v4(),
            book_file_id: Uuid::new_v4(),
            location: "epubcfi(/6/10)".into(),
            label: Some("Important passage".into()),
            excerpt: Some("It was the best of times...".into()),
            position: 0.25,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&bookmark).unwrap();
        let deserialized: Bookmark = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, bookmark);
    }
}
