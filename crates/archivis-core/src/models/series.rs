use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A book series (e.g., "The Lord of the Rings", "Discworld").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Series {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
}

impl Series {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_series() {
        let series = Series::new("Discworld");
        assert_eq!(series.name, "Discworld");
        assert!(series.description.is_none());
    }

    #[test]
    fn series_serde_roundtrip() {
        let series = Series {
            id: Uuid::new_v4(),
            name: "Dune Chronicles".into(),
            description: Some("Frank Herbert's epic saga".into()),
        };
        let json = serde_json::to_string(&series).unwrap();
        let deserialized: Series = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, series);
    }
}
