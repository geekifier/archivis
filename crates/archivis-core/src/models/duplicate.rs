use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a duplicate link in the review workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateStatus {
    /// Awaiting user review.
    Pending,
    /// Books have been merged.
    Merged,
    /// User dismissed (not a duplicate).
    Dismissed,
}

impl fmt::Display for DuplicateStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Merged => write!(f, "merged"),
            Self::Dismissed => write!(f, "dismissed"),
        }
    }
}

impl FromStr for DuplicateStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "merged" => Ok(Self::Merged),
            "dismissed" => Ok(Self::Dismissed),
            other => Err(format!("unknown duplicate status: {other}")),
        }
    }
}

/// A detected duplicate relationship between two books.
///
/// Created during import when fuzzy matching detects a potential duplicate,
/// or manually flagged by a user. Tracks the detection method and confidence
/// score for later review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateLink {
    pub id: Uuid,
    pub book_id_a: Uuid,
    pub book_id_b: Uuid,
    pub detection_method: String, // "hash", "isbn", "fuzzy", "user"
    pub confidence: f32,
    pub status: DuplicateStatus,
    pub created_at: DateTime<Utc>,
}

impl DuplicateLink {
    /// Create a new pending duplicate link.
    pub fn new(
        book_id_a: Uuid,
        book_id_b: Uuid,
        detection_method: impl Into<String>,
        confidence: f32,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            book_id_a,
            book_id_b,
            detection_method: detection_method.into(),
            confidence: confidence.clamp(0.0, 1.0),
            status: DuplicateStatus::Pending,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_status_display_and_parse() {
        assert_eq!(DuplicateStatus::Pending.to_string(), "pending");
        assert_eq!(DuplicateStatus::Merged.to_string(), "merged");
        assert_eq!(DuplicateStatus::Dismissed.to_string(), "dismissed");
        assert_eq!(
            "pending".parse::<DuplicateStatus>().unwrap(),
            DuplicateStatus::Pending,
        );
        assert_eq!(
            "merged".parse::<DuplicateStatus>().unwrap(),
            DuplicateStatus::Merged,
        );
        assert_eq!(
            "dismissed".parse::<DuplicateStatus>().unwrap(),
            DuplicateStatus::Dismissed,
        );
        assert!("bogus".parse::<DuplicateStatus>().is_err());
    }

    #[test]
    fn duplicate_status_serde_roundtrip() {
        let status = DuplicateStatus::Merged;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""merged""#);
        let deserialized: DuplicateStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, status);
    }

    #[test]
    fn new_duplicate_link_has_defaults() {
        let book_a = Uuid::new_v4();
        let book_b = Uuid::new_v4();
        let link = DuplicateLink::new(book_a, book_b, "fuzzy", 0.85);
        assert_eq!(link.book_id_a, book_a);
        assert_eq!(link.book_id_b, book_b);
        assert_eq!(link.detection_method, "fuzzy");
        assert!((link.confidence - 0.85).abs() < f32::EPSILON);
        assert_eq!(link.status, DuplicateStatus::Pending);
    }

    #[test]
    fn confidence_is_clamped() {
        let link = DuplicateLink::new(Uuid::new_v4(), Uuid::new_v4(), "fuzzy", 1.5);
        assert!((link.confidence - 1.0).abs() < f32::EPSILON);

        let link2 = DuplicateLink::new(Uuid::new_v4(), Uuid::new_v4(), "fuzzy", -0.5);
        assert!((link2.confidence - 0.0).abs() < f32::EPSILON);
    }
}
