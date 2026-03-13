use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Series information included in a candidate response.
#[derive(Debug, Serialize, ToSchema)]
pub struct SeriesInfo {
    pub name: String,
    pub position: Option<f32>,
}

/// An author entry in a candidate response, preserving the contributor role.
#[derive(Debug, Serialize, ToSchema)]
pub struct CandidateAuthor {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// A metadata resolution candidate returned by a provider.
#[derive(Debug, Serialize, ToSchema)]
pub struct CandidateResponse {
    pub id: Uuid,
    pub run_id: Option<Uuid>,
    pub provider_name: String,
    pub score: f32,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub authors: Vec<CandidateAuthor>,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub publication_year: Option<i32>,
    pub language: Option<String>,
    pub language_label: Option<String>,
    pub page_count: Option<i32>,
    pub isbn: Option<String>,
    pub series: Option<SeriesInfo>,
    pub cover_url: Option<String>,
    pub match_reasons: Vec<String>,
    pub disputes: Vec<String>,
    pub status: String,
    pub tier: Option<String>,
}

/// Response from triggering metadata refresh for a book.
#[derive(Debug, Serialize, ToSchema)]
pub struct RefreshMetadataResponse {
    /// Background task ID for SSE progress tracking.
    pub task_id: Uuid,
}

/// Request body for batch metadata refresh.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchRefreshMetadataRequest {
    pub book_ids: Vec<Uuid>,
}

/// Request body for refreshing the queued resolution backlog.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RefreshAllMetadataRequest {
    /// Maximum number of books to refresh (default: 100).
    pub max_books: Option<i64>,
}

/// Optional request body for the apply-candidate endpoint.
///
/// When omitted or empty, all fields are applied (default behavior).
#[derive(Debug, Deserialize, Default, ToSchema)]
pub struct ApplyCandidateBody {
    /// Fields to exclude from application (e.g. `["cover", "title"]`).
    #[serde(default)]
    pub exclude_fields: Vec<String>,
}

/// Request body for batch-rejecting candidates.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchRejectRequest {
    /// IDs of the candidates to reject.
    pub candidate_ids: Vec<Uuid>,
}

/// Response from the bulk metadata refresh endpoint.
#[derive(Debug, Serialize, ToSchema)]
pub struct RefreshAllMetadataResponse {
    /// Number of books queued for refresh.
    pub count: usize,
    /// Task IDs for SSE progress tracking.
    pub task_ids: Vec<Uuid>,
}
