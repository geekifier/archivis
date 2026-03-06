use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::enums::{MetadataSource, MetadataStatus, ResolutionOutcome, ResolutionState};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataProvenance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<FieldProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<FieldProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<FieldProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<FieldProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<FieldProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<FieldProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<FieldProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<FieldProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_count: Option<FieldProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover: Option<FieldProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldProvenance {
    pub origin: MetadataSource,
    pub protected: bool,
}

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
    /// Import-time local metadata quality score (0.0–1.0).
    #[serde(default)]
    pub ingest_quality_score: f32,
    #[serde(default)]
    pub resolution_state: ResolutionState,
    pub resolution_outcome: Option<ResolutionOutcome>,
    #[serde(default = "default_resolution_requested_at")]
    pub resolution_requested_at: DateTime<Utc>,
    pub resolution_requested_reason: Option<String>,
    pub last_resolved_at: Option<DateTime<Utc>>,
    pub last_resolution_run_id: Option<Uuid>,
    #[serde(default)]
    pub metadata_locked: bool,
    #[serde(default)]
    pub metadata_provenance: MetadataProvenance,
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
            ingest_quality_score: 0.0,
            resolution_state: ResolutionState::Pending,
            resolution_outcome: None,
            resolution_requested_at: now,
            resolution_requested_reason: None,
            last_resolved_at: None,
            last_resolution_run_id: None,
            metadata_locked: false,
            metadata_provenance: MetadataProvenance::default(),
            cover_path: None,
        }
    }
}

fn default_resolution_requested_at() -> DateTime<Utc> {
    Utc::now()
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
        assert!((book.ingest_quality_score - 0.0).abs() < f32::EPSILON);
        assert_eq!(book.resolution_state, ResolutionState::Pending);
        assert!(book.resolution_outcome.is_none());
        assert!(book.resolution_requested_reason.is_none());
        assert!(book.last_resolved_at.is_none());
        assert!(book.last_resolution_run_id.is_none());
        assert!(!book.metadata_locked);
        assert_eq!(book.metadata_provenance, MetadataProvenance::default());
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

    #[test]
    fn metadata_provenance_is_sparse() {
        let provenance = MetadataProvenance {
            title: Some(FieldProvenance {
                origin: MetadataSource::User,
                protected: true,
            }),
            ..MetadataProvenance::default()
        };

        let json = serde_json::to_string(&provenance).unwrap();
        assert_eq!(
            json,
            r#"{"title":{"origin":{"type":"user"},"protected":true}}"#
        );
    }
}
