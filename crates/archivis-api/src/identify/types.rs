use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Series information included in a candidate response.
#[derive(Debug, Serialize, ToSchema)]
pub struct SeriesInfo {
    pub name: String,
    pub position: Option<f32>,
}

/// A metadata identification candidate returned by a provider.
#[derive(Debug, Serialize, ToSchema)]
pub struct CandidateResponse {
    pub id: Uuid,
    pub provider_name: String,
    pub score: f32,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub publication_date: Option<String>,
    pub isbn: Option<String>,
    pub series: Option<SeriesInfo>,
    pub cover_url: Option<String>,
    pub match_reasons: Vec<String>,
    pub status: String,
}

/// Response from triggering identification for a book.
#[derive(Debug, Serialize, ToSchema)]
pub struct IdentifyResponse {
    /// Background task ID for SSE progress tracking.
    pub task_id: Uuid,
}

/// Request body for batch identification.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchIdentifyRequest {
    pub book_ids: Vec<Uuid>,
}

/// Request body for identifying all unidentified books.
#[derive(Debug, Deserialize, ToSchema)]
pub struct IdentifyAllRequest {
    /// Maximum number of books to identify (default: 100).
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

/// Response from the identify-all endpoint.
#[derive(Debug, Serialize, ToSchema)]
pub struct IdentifyAllResponse {
    /// Number of books queued for identification.
    pub count: usize,
    /// Task IDs for SSE progress tracking.
    pub task_ids: Vec<Uuid>,
}
