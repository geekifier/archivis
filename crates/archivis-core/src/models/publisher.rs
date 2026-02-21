use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A book publisher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publisher {
    pub id: Uuid,
    pub name: String,
}

impl Publisher {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_publisher() {
        let pub_ = Publisher::new("Tor Books");
        assert_eq!(pub_.name, "Tor Books");
    }

    #[test]
    fn publisher_serde_roundtrip() {
        let pub_ = Publisher::new("Penguin Random House");
        let json = serde_json::to_string(&pub_).unwrap();
        let deserialized: Publisher = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, pub_);
    }
}
