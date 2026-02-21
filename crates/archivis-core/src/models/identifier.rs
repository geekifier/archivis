use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::enums::{IdentifierType, MetadataSource};

/// An external identifier linked to a book (ISBN, ASIN, etc.).
/// Multiple identifiers can reference the same book (different editions,
/// different provider IDs). ISBNs are evidence, not primary keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identifier {
    pub id: Uuid,
    pub book_id: Uuid,
    pub identifier_type: IdentifierType,
    pub value: String,
    /// Where this identifier was discovered.
    pub source: MetadataSource,
    /// Confidence that this identifier correctly references this book (0.0–1.0).
    pub confidence: f32,
}

impl Identifier {
    pub fn new(
        book_id: Uuid,
        identifier_type: IdentifierType,
        value: impl Into<String>,
        source: MetadataSource,
        confidence: f32,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            book_id,
            identifier_type,
            value: value.into(),
            source,
            confidence: confidence.clamp(0.0, 1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_identifier() {
        let book_id = Uuid::new_v4();
        let id = Identifier::new(
            book_id,
            IdentifierType::Isbn13,
            "978-0-441-17271-9",
            MetadataSource::Embedded,
            0.95,
        );
        assert_eq!(id.book_id, book_id);
        assert_eq!(id.identifier_type, IdentifierType::Isbn13);
        assert_eq!(id.value, "978-0-441-17271-9");
    }

    #[test]
    fn confidence_clamped() {
        let id = Identifier::new(
            Uuid::new_v4(),
            IdentifierType::Asin,
            "B000FA5ZEG",
            MetadataSource::Provider("Amazon".into()),
            1.5,
        );
        assert!((id.confidence - 1.0).abs() < f32::EPSILON);

        let id2 = Identifier::new(
            Uuid::new_v4(),
            IdentifierType::Isbn10,
            "0441172717",
            MetadataSource::Filename,
            -0.1,
        );
        assert!((id2.confidence - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn identifier_serde_roundtrip() {
        let id = Identifier::new(
            Uuid::new_v4(),
            IdentifierType::OpenLibrary,
            "OL123456W",
            MetadataSource::Provider("Open Library".into()),
            0.8,
        );
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: Identifier = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, id);
    }
}
