use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An author who can be associated with one or more books.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub id: Uuid,
    pub name: String,
    /// Name formatted for alphabetical sorting (e.g., "Herbert, Frank").
    pub sort_name: String,
}

impl Author {
    /// Create a new `Author`, auto-generating a sort name from "First Last" → "Last, First".
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let sort_name = generate_sort_name(&name);
        Self {
            id: Uuid::new_v4(),
            name,
            sort_name,
        }
    }
}

/// Generate a sortable name: "First Middle Last" → "Last, First Middle".
/// Names with no spaces are returned unchanged.
fn generate_sort_name(name: &str) -> String {
    let trimmed = name.trim();
    match trimmed.rsplit_once(' ') {
        Some((first_parts, last)) => format!("{last}, {first_parts}"),
        None => trimmed.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_author_generates_sort_name() {
        let author = Author::new("Frank Herbert");
        assert_eq!(author.name, "Frank Herbert");
        assert_eq!(author.sort_name, "Herbert, Frank");
    }

    #[test]
    fn sort_name_single_name() {
        assert_eq!(generate_sort_name("Plato"), "Plato");
    }

    #[test]
    fn sort_name_multi_part() {
        assert_eq!(generate_sort_name("J. R. R. Tolkien"), "Tolkien, J. R. R.",);
    }

    #[test]
    fn author_serde_roundtrip() {
        let author = Author::new("Ursula K. Le Guin");
        let json = serde_json::to_string(&author).unwrap();
        let deserialized: Author = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, author);
    }
}
