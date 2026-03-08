use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::book::FieldProvenance;

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
    pub tier: Option<String>,
    pub status: CandidateStatus,
    pub created_at: DateTime<Utc>,
    /// JSON-serialized [`ApplyChangeset`] recording what the apply changed.
    /// Present only while the apply is undoable.
    pub apply_changeset: Option<serde_json::Value>,
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
            tier: None,
            status: CandidateStatus::Pending,
            created_at: Utc::now(),
            apply_changeset: None,
        }
    }
}

/// Records pre-apply state for each field that was actually mutated,
/// enabling provenance-guarded undo.
///
/// Only populated fields were changed by the apply. Each present field
/// holds the **pre-apply value** and **pre-apply provenance** so undo
/// can restore them (after verifying the field hasn't been edited since).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyChangeset {
    /// Provider name that performed this apply (for provenance matching on undo).
    pub provider_name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<ChangesetEntry<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_title: Option<ChangesetEntry<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<ChangesetEntry<Option<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<ChangesetEntry<Option<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<ChangesetEntry<Option<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<ChangesetEntry<Option<NaiveDate>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_count: Option<ChangesetEntry<Option<i32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_path: Option<ChangesetEntry<Option<String>>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<ChangesetEntry<Vec<ChangesetAuthor>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<ChangesetEntry<Vec<ChangesetSeries>>>,
}

impl ApplyChangeset {
    /// True if no fields were actually changed by the apply.
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.sort_title.is_none()
            && self.subtitle.is_none()
            && self.description.is_none()
            && self.language.is_none()
            && self.publication_date.is_none()
            && self.page_count.is_none()
            && self.cover_path.is_none()
            && self.authors.is_none()
            && self.series.is_none()
    }
}

/// A field's pre-apply value and its pre-apply provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetEntry<T> {
    pub old_value: T,
    pub old_provenance: Option<FieldProvenance>,
}

/// Snapshot of a book-author link for changeset storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetAuthor {
    pub author_id: Uuid,
    pub name: String,
    pub sort_name: String,
    pub role: String,
    pub position: i64,
}

/// Snapshot of a book-series link for changeset storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetSeries {
    pub series_id: Uuid,
    pub name: String,
    pub position: Option<f64>,
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
