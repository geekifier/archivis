use archivis_core::models::IdentifierType;
use serde::{Deserialize, Serialize};

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
    pub publication_date: Option<String>,
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

/// Series information from a metadata provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSeries {
    pub name: String,
    pub position: Option<f32>,
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
            publication_date: Some("1965".to_string()),
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
            publication_date: None,
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
}
