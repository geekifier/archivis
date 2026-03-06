use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of an identification candidate in the review workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    /// Awaiting user review.
    Pending,
    /// Metadata has been applied to the book.
    Applied,
    /// User rejected this candidate.
    Rejected,
    /// Candidate was invalidated by a newer resolution run.
    Superseded,
}

impl fmt::Display for CandidateStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Applied => write!(f, "applied"),
            Self::Rejected => write!(f, "rejected"),
            Self::Superseded => write!(f, "superseded"),
        }
    }
}

impl FromStr for CandidateStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "applied" => Ok(Self::Applied),
            "rejected" => Ok(Self::Rejected),
            "superseded" => Ok(Self::Superseded),
            other => Err(format!("unknown candidate status: {other}")),
        }
    }
}

/// A metadata identification candidate stored for user review.
///
/// Each candidate represents a potential match from an external metadata
/// provider, with a confidence score and the full provider metadata
/// serialized as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentificationCandidate {
    pub id: Uuid,
    pub book_id: Uuid,
    pub run_id: Option<Uuid>,
    pub provider_name: String,
    pub score: f32,
    /// Serialized `ProviderMetadata` from the metadata provider.
    pub metadata: serde_json::Value,
    pub match_reasons: Vec<String>,
    #[serde(default)]
    pub disputes: Vec<String>,
    pub status: CandidateStatus,
    pub created_at: DateTime<Utc>,
}

impl IdentificationCandidate {
    /// Create a new pending candidate.
    pub fn new(
        book_id: Uuid,
        provider_name: impl Into<String>,
        score: f32,
        metadata: serde_json::Value,
        match_reasons: Vec<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            book_id,
            run_id: None,
            provider_name: provider_name.into(),
            score: score.clamp(0.0, 1.0),
            metadata,
            match_reasons,
            disputes: Vec::new(),
            status: CandidateStatus::Pending,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_status_display_and_parse() {
        assert_eq!(CandidateStatus::Pending.to_string(), "pending");
        assert_eq!(CandidateStatus::Applied.to_string(), "applied");
        assert_eq!(CandidateStatus::Rejected.to_string(), "rejected");
        assert_eq!(CandidateStatus::Superseded.to_string(), "superseded");
        assert_eq!(
            "pending".parse::<CandidateStatus>().unwrap(),
            CandidateStatus::Pending,
        );
        assert_eq!(
            "applied".parse::<CandidateStatus>().unwrap(),
            CandidateStatus::Applied,
        );
        assert_eq!(
            "rejected".parse::<CandidateStatus>().unwrap(),
            CandidateStatus::Rejected,
        );
        assert_eq!(
            "superseded".parse::<CandidateStatus>().unwrap(),
            CandidateStatus::Superseded,
        );
        assert!("bogus".parse::<CandidateStatus>().is_err());
    }

    #[test]
    fn candidate_status_serde_roundtrip() {
        let status = CandidateStatus::Applied;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""applied""#);
        let deserialized: CandidateStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, status);
    }

    #[test]
    fn new_candidate_has_defaults() {
        let book_id = Uuid::new_v4();
        let metadata = serde_json::json!({
            "provider_name": "open_library",
            "title": "Dune",
        });
        let candidate = IdentificationCandidate::new(
            book_id,
            "open_library",
            0.95,
            metadata,
            vec!["ISBN exact match".into()],
        );
        assert_eq!(candidate.book_id, book_id);
        assert_eq!(candidate.provider_name, "open_library");
        assert!((candidate.score - 0.95).abs() < f32::EPSILON);
        assert_eq!(candidate.status, CandidateStatus::Pending);
        assert_eq!(candidate.match_reasons.len(), 1);
        assert!(candidate.run_id.is_none());
        assert!(candidate.disputes.is_empty());
    }

    #[test]
    fn score_is_clamped() {
        let metadata = serde_json::json!({});
        let candidate = IdentificationCandidate::new(Uuid::new_v4(), "test", 1.5, metadata, vec![]);
        assert!((candidate.score - 1.0).abs() < f32::EPSILON);

        let metadata2 = serde_json::json!({});
        let candidate2 =
            IdentificationCandidate::new(Uuid::new_v4(), "test", -0.5, metadata2, vec![]);
        assert!((candidate2.score - 0.0).abs() < f32::EPSILON);
    }
}
