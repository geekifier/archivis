use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::enums::MetadataStatus;

/// A logical book in the library. May have multiple associated files (formats),
/// authors, series memberships, and identifiers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Book {
    pub id: Uuid,
    pub title: String,
    pub subtitle: Option<String>,
    pub sort_title: String,
    pub description: Option<String>,
    pub language: Option<String>,
    pub publication_date: Option<NaiveDate>,
    pub publisher_id: Option<Uuid>,
    pub added_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// User rating on a 0.0–5.0 scale (half-star increments).
    pub rating: Option<f32>,
    pub page_count: Option<i32>,
    pub metadata_status: MetadataStatus,
    /// Confidence score from the metadata identification pipeline (0.0–1.0).
    pub metadata_confidence: f32,
    /// Path to the primary cover image in storage, if available.
    pub cover_path: Option<String>,
}

impl Book {
    /// Update the title, keeping `sort_title` in sync.
    pub fn set_title(&mut self, title: impl Into<String>) {
        let title = title.into();
        self.sort_title = generate_sort_title(&title);
        self.title = title;
    }

    /// Create a new `Book` with required fields and sensible defaults.
    pub fn new(title: impl Into<String>) -> Self {
        let title = title.into();
        let sort_title = generate_sort_title(&title);
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title,
            subtitle: None,
            sort_title,
            description: None,
            language: None,
            publication_date: None,
            publisher_id: None,
            added_at: now,
            updated_at: now,
            rating: None,
            page_count: None,
            metadata_status: MetadataStatus::Unidentified,
            metadata_confidence: 0.0,
            cover_path: None,
        }
    }
}

/// Generate a sort-friendly title by stripping leading articles.
pub fn generate_sort_title(title: &str) -> String {
    let lower = title.to_lowercase();
    for article in ["the ", "a ", "an "] {
        if lower.starts_with(article) {
            return title[article.len()..].to_string();
        }
    }
    title.to_string()
}

/// Leading articles stripped during title normalization for duplicate detection.
const ARTICLES: &[&str] = &[
    "the", "a", "an", // English
    "der", "die", "das", // German
    "le", "la", "les", // French
];

/// Normalize a book title for comparison and DB storage (`norm_title` column).
///
/// Lowercase, remove articles (`the`/`a`/`an`/`der`/`die`/`das`/`le`/`la`/`les`),
/// remove punctuation, collapse whitespace, trim.
pub fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase();

    // Replace punctuation: apostrophes are dropped (contractions stay
    // joined), all other non-alphanumeric characters become spaces.
    let cleaned: String = lower
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == ' ' {
                Some(c)
            } else if c == '\'' || c == '\u{2019}' {
                None
            } else {
                Some(' ')
            }
        })
        .collect();

    let words: Vec<&str> = cleaned.split_whitespace().collect();

    // Strip leading article if present (only when there are more words).
    let words = if words.len() > 1 && ARTICLES.contains(&words[0]) {
        &words[1..]
    } else {
        &words
    };

    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_book_has_defaults() {
        let book = Book::new("Dune");
        assert_eq!(book.title, "Dune");
        assert_eq!(book.sort_title, "Dune");
        assert_eq!(book.metadata_status, MetadataStatus::Unidentified);
        assert!((book.metadata_confidence - 0.0).abs() < f32::EPSILON);
        assert!(book.description.is_none());
        assert!(book.rating.is_none());
    }

    #[test]
    fn sort_title_strips_articles() {
        assert_eq!(generate_sort_title("The Hobbit"), "Hobbit");
        assert_eq!(generate_sort_title("A Game of Thrones"), "Game of Thrones");
        assert_eq!(generate_sort_title("An Introduction"), "Introduction");
        assert_eq!(generate_sort_title("Dune"), "Dune");
        // Case-insensitive
        assert_eq!(
            generate_sort_title("the lord of the rings"),
            "lord of the rings"
        );
    }

    #[test]
    fn book_serde_roundtrip() {
        let book = Book::new("The Hitchhiker's Guide to the Galaxy");
        let json = serde_json::to_string(&book).unwrap();
        let deserialized: Book = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.title, book.title);
        assert_eq!(deserialized.sort_title, book.sort_title);
        assert_eq!(deserialized.id, book.id);
    }
}
