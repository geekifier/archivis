use std::borrow::Cow;

use archivis_core::isbn::{validate_isbn, IsbnType};
use archivis_core::models::IdentifierType;
use serde::{Deserialize, Serialize};

// ── Provider capabilities ───────────────────────────────────────────

/// Provenance-based quality tier for a metadata provider.
///
/// Used as a static bias signal by the resolver and displayed as star
/// ratings in the admin UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ProviderQuality {
    /// Crowdsourced, variable quality (Open Library, Goodreads).
    Community = 1,
    /// Commercially/professionally maintained (Hardcover, Google Books).
    Curated = 2,
    /// Institutional cataloging standards (LOC, `WorldCat`).
    Authoritative = 3,
}

/// Extensible capability flags for metadata providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderFeature {
    /// Free-text title/author search.
    Search,
    /// Can supply cover image URLs.
    Covers,
}

/// Compile-time capabilities advertised by a metadata provider.
///
/// All references are `&'static` because capabilities are constants.
#[derive(Debug)]
pub struct ProviderCapabilities {
    pub quality: ProviderQuality,
    /// Provider's documented/default rate limit (requests per minute).
    pub default_rate_limit_rpm: u32,
    /// Identifier types this provider can look up.
    pub supported_id_lookups: &'static [IdentifierType],
    /// Feature flags (search, covers, etc.).
    pub features: &'static [ProviderFeature],
}

impl ProviderCapabilities {
    /// Check whether the provider advertises the given feature.
    pub fn has_feature(&self, feature: ProviderFeature) -> bool {
        self.features.contains(&feature)
    }

    /// Check whether the provider supports lookup by the given identifier type.
    pub fn supports_id_lookup(&self, id_type: IdentifierType) -> bool {
        self.supported_id_lookups.contains(&id_type)
    }

    /// Check whether the provider supports ISBN lookup (either ISBN-10 or ISBN-13).
    pub fn supports_isbn(&self) -> bool {
        self.supports_id_lookup(IdentifierType::Isbn13)
            || self.supports_id_lookup(IdentifierType::Isbn10)
    }
}

/// Metadata result returned by a provider lookup.
/// Fields are all optional — providers may have partial data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMetadata {
    /// Provider name, e.g. `"open_library"`, `"hardcover"`.
    pub provider_name: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub authors: Vec<ProviderAuthor>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub publication_year: Option<i32>,
    /// External identifiers (ISBNs, OLIDs, etc.).
    pub identifiers: Vec<ProviderIdentifier>,
    /// Genres/tags/subjects.
    pub subjects: Vec<String>,
    pub series: Option<ProviderSeries>,
    pub page_count: Option<i32>,
    /// URL to download cover image.
    pub cover_url: Option<String>,
    pub rating: Option<f32>,
    /// Physical format of the edition (e.g. `"Paperback"`, `"Audio CD"`).
    pub physical_format: Option<String>,
    /// Provider's self-assessed match confidence (0.0-1.0).
    pub confidence: f32,
}

/// An author as returned by a metadata provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAuthor {
    pub name: String,
    /// Role such as "author", "editor", "translator", etc.
    pub role: Option<String>,
}

/// An external identifier returned by a metadata provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderIdentifier {
    pub identifier_type: IdentifierType,
    pub value: String,
}

impl ProviderIdentifier {
    /// Create a validated ISBN identifier from a raw string.
    ///
    /// Strips all non-digit/X characters, validates via `validate_isbn()`,
    /// and returns `None` if the result is not a valid ISBN-10 or ISBN-13.
    pub fn isbn(raw: &str) -> Option<Self> {
        // Strip everything except digits and X
        let cleaned: String = raw
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
            .collect();

        let result = validate_isbn(&cleaned);
        if !result.valid {
            return None;
        }

        let identifier_type = match result.isbn_type {
            Some(IsbnType::Isbn13) => IdentifierType::Isbn13,
            Some(IsbnType::Isbn10) => IdentifierType::Isbn10,
            None => return None,
        };

        Some(Self {
            identifier_type,
            value: result.normalized,
        })
    }
}

/// Series information from a metadata provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSeries {
    pub name: String,
    pub position: Option<f32>,
}

/// Extract a publication year from a date-like string.
///
/// Handles bare years (`"1965"`), ISO dates (`"2023-01-15"`), and
/// free-form strings such as `"June 1965"` by finding the first run of
/// four consecutive ASCII digits.
pub fn parse_year_from_str(s: &str) -> Option<i32> {
    let s = s.trim();
    // Try parsing as a bare number first.
    if let Ok(year) = s.parse::<i32>() {
        return Some(year);
    }
    // Extract first 4 consecutive digits.
    s.as_bytes()
        .windows(4)
        .find(|w| w.iter().all(u8::is_ascii_digit))
        .and_then(|w| std::str::from_utf8(w).ok())
        .and_then(|w| w.parse::<i32>().ok())
}

/// Minor words that should not be capitalized in title case (unless first word).
const TITLE_CASE_MINOR_WORDS: &[&str] = &[
    "a", "an", "the", "and", "but", "or", "nor", "for", "yet", "so", "at", "by", "in", "of", "on",
    "to", "up", "as", "is", "if", "it", "vs", "via",
];

/// Apply English title case to a string.
///
/// Lowercases all-caps input before applying rules. The first word is always
/// capitalized. Minor words (articles, conjunctions, short prepositions) are
/// lowercased unless they are the first word.
pub fn titlecase_title(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    // If the entire string is uppercase, lowercase it first
    let s: Cow<str> = if s.chars().any(char::is_alphabetic)
        && s.chars().all(|c| !c.is_alphabetic() || c.is_uppercase())
    {
        Cow::Owned(s.to_lowercase())
    } else {
        Cow::Borrowed(s)
    };

    let mut result = String::with_capacity(s.len());
    let mut first_word = true;

    for chunk in s.split_inclusive(char::is_whitespace) {
        let trimmed = chunk.trim();
        if trimmed.is_empty() {
            result.push_str(chunk);
            continue;
        }

        let lower = trimmed.to_lowercase();
        let capitalized = if first_word || !TITLE_CASE_MINOR_WORDS.contains(&lower.as_str()) {
            // Capitalize first letter
            let mut chars = trimmed.chars();
            chars.next().map_or_else(String::new, |c| {
                let mut word = c.to_uppercase().to_string();
                word.extend(chars);
                word
            })
        } else {
            lower
        };
        first_word = false;

        // Re-attach trailing whitespace from the chunk
        result.push_str(&capitalized);
        result.push_str(&chunk[chunk.trim_end().len()..]);
    }

    result
}

/// A search query for looking up book metadata.
#[derive(Debug, Clone, Default)]
pub struct MetadataQuery {
    /// ISBN-13 or ISBN-10.
    pub isbn: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub asin: Option<String>,
}

#[cfg(test)]
pub(crate) fn assert_isbns_valid(metadata: &ProviderMetadata) {
    for id in &metadata.identifiers {
        if matches!(
            id.identifier_type,
            IdentifierType::Isbn10 | IdentifierType::Isbn13
        ) {
            let result = validate_isbn(&id.value);
            assert!(
                result.valid,
                "invalid ISBN in provider output: {:?} (value={:?}, {})",
                id.identifier_type, id.value, result.message,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_metadata_serde_roundtrip() {
        let metadata = ProviderMetadata {
            provider_name: "open_library".to_string(),
            title: Some("Dune".to_string()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Frank Herbert".to_string(),
                role: Some("author".to_string()),
            }],
            description: Some("A science fiction classic.".to_string()),
            language: Some("en".to_string()),
            publisher: Some("Chilton Books".to_string()),
            publication_year: Some(1965),
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".to_string(),
            }],
            subjects: vec!["Science Fiction".to_string()],
            series: Some(ProviderSeries {
                name: "Dune".to_string(),
                position: Some(1.0),
            }),
            page_count: Some(412),
            cover_url: Some("https://covers.openlibrary.org/b/id/12345-L.jpg".to_string()),
            rating: Some(4.5),
            physical_format: None,
            confidence: 0.95,
        };

        let json = serde_json::to_string(&metadata).unwrap();
        let deserialized: ProviderMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.provider_name, "open_library");
        assert_eq!(deserialized.title.as_deref(), Some("Dune"));
        assert_eq!(deserialized.authors.len(), 1);
        assert_eq!(deserialized.authors[0].name, "Frank Herbert");
        assert_eq!(deserialized.identifiers.len(), 1);
        assert_eq!(
            deserialized.identifiers[0].identifier_type,
            IdentifierType::Isbn13
        );
        assert_eq!(deserialized.series.as_ref().unwrap().name, "Dune");
        assert!((deserialized.confidence - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn provider_metadata_minimal_serde() {
        let metadata = ProviderMetadata {
            provider_name: "test".to_string(),
            title: None,
            subtitle: None,
            authors: vec![],
            description: None,
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.0,
        };

        let json = serde_json::to_string(&metadata).unwrap();
        let deserialized: ProviderMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.provider_name, "test");
        assert!(deserialized.title.is_none());
        assert!(deserialized.authors.is_empty());
    }

    #[test]
    fn metadata_query_default() {
        let query = MetadataQuery::default();
        assert!(query.isbn.is_none());
        assert!(query.title.is_none());
        assert!(query.author.is_none());
        assert!(query.asin.is_none());
    }

    // ── titlecase_title ───────────────────────────────────────────

    #[test]
    fn titlecase_basic_cases() {
        assert_eq!(titlecase_title("lord of the rings"), "Lord of the Rings");
        assert_eq!(
            titlecase_title("the lord of the rings"),
            "The Lord of the Rings"
        );
        assert_eq!(
            titlecase_title("a tale of two cities"),
            "A Tale of Two Cities"
        );
        assert_eq!(titlecase_title("war and peace"), "War and Peace");
        assert_eq!(titlecase_title("dune"), "Dune");
        assert_eq!(titlecase_title(""), "");
    }

    #[test]
    fn titlecase_all_caps_input() {
        assert_eq!(
            titlecase_title("THE LORD OF THE RINGS"),
            "The Lord of the Rings"
        );
    }

    #[test]
    fn titlecase_single_word() {
        assert_eq!(titlecase_title("dune"), "Dune");
        assert_eq!(titlecase_title("the"), "The");
    }

    // ── ProviderIdentifier::isbn ──────────────────────────────────

    #[test]
    fn isbn_constructor_valid_isbn13() {
        let id = ProviderIdentifier::isbn("9780441172719").unwrap();
        assert_eq!(id.identifier_type, IdentifierType::Isbn13);
        assert_eq!(id.value, "9780441172719");
    }

    #[test]
    fn isbn_constructor_strips_marc_punctuation() {
        let id = ProviderIdentifier::isbn("0743535308 :").unwrap();
        assert_eq!(id.identifier_type, IdentifierType::Isbn10);
        assert_eq!(id.value, "0743535308");
    }

    #[test]
    fn isbn_constructor_strips_qualifier() {
        let id = ProviderIdentifier::isbn("978-0-7435-3530-4 (pbk.)").unwrap();
        assert_eq!(id.identifier_type, IdentifierType::Isbn13);
        assert_eq!(id.value, "9780743535304");
    }

    #[test]
    fn isbn_constructor_invalid_returns_none() {
        assert!(ProviderIdentifier::isbn("not-an-isbn").is_none());
        assert!(ProviderIdentifier::isbn("").is_none());
    }

    // ── ProviderQuality ─────────────────────────────────────────────

    #[test]
    fn provider_quality_discriminants() {
        assert_eq!(ProviderQuality::Community as u8, 1);
        assert_eq!(ProviderQuality::Curated as u8, 2);
        assert_eq!(ProviderQuality::Authoritative as u8, 3);
    }

    // ── ProviderCapabilities helpers ────────────────────────────────

    #[test]
    fn capabilities_has_feature() {
        static CAPS: ProviderCapabilities = ProviderCapabilities {
            quality: ProviderQuality::Community,
            default_rate_limit_rpm: 100,
            supported_id_lookups: &[],
            features: &[ProviderFeature::Search],
        };
        assert!(CAPS.has_feature(ProviderFeature::Search));
        assert!(!CAPS.has_feature(ProviderFeature::Covers));
    }

    #[test]
    fn capabilities_supports_id_lookup() {
        static CAPS: ProviderCapabilities = ProviderCapabilities {
            quality: ProviderQuality::Curated,
            default_rate_limit_rpm: 50,
            supported_id_lookups: &[IdentifierType::Isbn13, IdentifierType::Asin],
            features: &[],
        };
        assert!(CAPS.supports_id_lookup(IdentifierType::Isbn13));
        assert!(CAPS.supports_id_lookup(IdentifierType::Asin));
        assert!(!CAPS.supports_id_lookup(IdentifierType::Lccn));
    }

    static ISBN13_ONLY_CAPS: ProviderCapabilities = ProviderCapabilities {
        quality: ProviderQuality::Authoritative,
        default_rate_limit_rpm: 20,
        supported_id_lookups: &[IdentifierType::Isbn13],
        features: &[],
    };

    static ISBN10_ONLY_CAPS: ProviderCapabilities = ProviderCapabilities {
        quality: ProviderQuality::Community,
        default_rate_limit_rpm: 100,
        supported_id_lookups: &[IdentifierType::Isbn10],
        features: &[],
    };

    static NO_ISBN_CAPS: ProviderCapabilities = ProviderCapabilities {
        quality: ProviderQuality::Curated,
        default_rate_limit_rpm: 50,
        supported_id_lookups: &[IdentifierType::Asin],
        features: &[],
    };

    #[test]
    fn capabilities_supports_isbn() {
        assert!(ISBN13_ONLY_CAPS.supports_isbn());
        assert!(ISBN10_ONLY_CAPS.supports_isbn());
        assert!(!NO_ISBN_CAPS.supports_isbn());
    }
}
