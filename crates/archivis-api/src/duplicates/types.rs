use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::books::types::BookSummary;

/// A duplicate link enriched with book summaries for both sides.
#[derive(Debug, Serialize, ToSchema)]
pub struct DuplicateLinkResponse {
    pub id: Uuid,
    pub book_a: BookSummary,
    pub book_b: BookSummary,
    pub detection_method: String,
    pub confidence: f32,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

/// Paginated list of duplicate links.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaginatedDuplicates {
    pub items: Vec<DuplicateLinkResponse>,
    pub total: u32,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
}

/// Request body for `POST /api/duplicates/{id}/merge`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MergeRequest {
    /// Which book to keep (the primary).
    pub primary_id: Uuid,
    /// Which book to absorb and delete (the secondary).
    pub secondary_id: Uuid,
    /// Metadata preference: "primary", "secondary", or "`higher_confidence`".
    pub prefer_metadata_from: Option<String>,
}

/// Request body for `POST /api/books/{id}/duplicates`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FlagDuplicateRequest {
    /// The other book that is a duplicate.
    pub other_book_id: Uuid,
}

/// Query parameters for `GET /api/duplicates`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct DuplicateListParams {
    /// Page number (1-indexed).
    pub page: Option<u32>,
    /// Results per page (max 100).
    pub per_page: Option<u32>,
}

/// Response for `GET /api/duplicates/count`.
#[derive(Debug, Serialize, ToSchema)]
pub struct DuplicateCountResponse {
    pub count: i64,
}
