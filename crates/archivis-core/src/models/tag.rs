use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A tag that can be applied to books for categorization.
/// Tags optionally belong to a category (e.g., category "genre" → tag "science fiction").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub id: Uuid,
    pub name: String,
    pub category: Option<String>,
}

impl Tag {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            category: None,
        }
    }

    pub fn with_category(name: impl Into<String>, category: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            category: Some(category.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tag() {
        let tag = Tag::new("science fiction");
        assert_eq!(tag.name, "science fiction");
        assert!(tag.category.is_none());
    }

    #[test]
    fn tag_with_category() {
        let tag = Tag::with_category("science fiction", "genre");
        assert_eq!(tag.name, "science fiction");
        assert_eq!(tag.category.as_deref(), Some("genre"));
    }

    #[test]
    fn tag_serde_roundtrip() {
        let tag = Tag::with_category("hard sci-fi", "genre");
        let json = serde_json::to_string(&tag).unwrap();
        let deserialized: Tag = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, tag);
    }
}
