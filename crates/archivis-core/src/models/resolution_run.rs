use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ResolutionOutcome;

/// Lifecycle state for a durable resolution run row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionRunState {
    #[default]
    Running,
    Done,
    Failed,
    Superseded,
}

impl fmt::Display for ResolutionRunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Done => write!(f, "done"),
            Self::Failed => write!(f, "failed"),
            Self::Superseded => write!(f, "superseded"),
        }
    }
}

impl FromStr for ResolutionRunState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "running" => Ok(Self::Running),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            "superseded" => Ok(Self::Superseded),
            _ => Err(format!("unknown resolution run state: {s}")),
        }
    }
}

/// Durable history row for a single provider-resolution attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolutionRun {
    pub id: Uuid,
    pub book_id: Uuid,
    pub trigger: String,
    pub state: ResolutionRunState,
    pub outcome: Option<ResolutionOutcome>,
    pub query_json: serde_json::Value,
    pub decision_code: String,
    pub candidate_count: i64,
    pub best_candidate_id: Option<Uuid>,
    pub best_score: Option<f32>,
    pub best_tier: Option<String>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl ResolutionRun {
    /// Create a new running resolution run with schema-aligned defaults.
    pub fn new(book_id: Uuid, trigger: impl Into<String>, query_json: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            book_id,
            trigger: trigger.into(),
            state: ResolutionRunState::Running,
            outcome: None,
            query_json,
            decision_code: "running".into(),
            candidate_count: 0,
            best_candidate_id: None,
            best_score: None,
            best_tier: None,
            error: None,
            started_at: Utc::now(),
            finished_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_run_state_roundtrip() {
        assert_eq!(ResolutionRunState::Running.to_string(), "running");
        assert_eq!(ResolutionRunState::Done.to_string(), "done");
        assert_eq!(ResolutionRunState::Failed.to_string(), "failed");
        assert_eq!(ResolutionRunState::Superseded.to_string(), "superseded");

        assert_eq!(
            "running".parse::<ResolutionRunState>().unwrap(),
            ResolutionRunState::Running
        );
        assert_eq!(
            "done".parse::<ResolutionRunState>().unwrap(),
            ResolutionRunState::Done
        );
        assert_eq!(
            "failed".parse::<ResolutionRunState>().unwrap(),
            ResolutionRunState::Failed
        );
        assert_eq!(
            "superseded".parse::<ResolutionRunState>().unwrap(),
            ResolutionRunState::Superseded
        );
        assert!("bogus".parse::<ResolutionRunState>().is_err());

        let json = serde_json::to_string(&ResolutionRunState::Superseded).unwrap();
        assert_eq!(json, r#""superseded""#);
        let roundtrip: ResolutionRunState = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, ResolutionRunState::Superseded);
    }

    #[test]
    fn new_run_has_running_defaults() {
        let book_id = Uuid::new_v4();
        let query_json = serde_json::json!({"title":"Dune"});
        let run = ResolutionRun::new(book_id, "manual_refresh", query_json.clone());

        assert_eq!(run.book_id, book_id);
        assert_eq!(run.trigger, "manual_refresh");
        assert_eq!(run.state, ResolutionRunState::Running);
        assert!(run.outcome.is_none());
        assert_eq!(run.query_json, query_json);
        assert_eq!(run.decision_code, "running");
        assert_eq!(run.candidate_count, 0);
        assert!(run.best_candidate_id.is_none());
        assert!(run.best_score.is_none());
        assert!(run.best_tier.is_none());
        assert!(run.error.is_none());
        assert!(run.finished_at.is_none());
    }
}
