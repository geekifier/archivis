use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use archivis_core::models::Publisher;
use archivis_db::PaginatedResult;

// ── Query Parameters ────────────────────────────────────────────

/// Query parameters for `GET /api/publishers`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct PublisherListParams {
    /// Page number (1-indexed).
    pub page: Option<u32>,
    /// Results per page (max 100).
    pub per_page: Option<u32>,
    /// Sort field: `name` (default).
    pub sort_by: Option<String>,
    /// Sort direction: asc (default) or desc.
    pub sort_order: Option<String>,
    /// Search by name (case-insensitive substring match).
    pub q: Option<String>,
}

/// Query parameters for `GET /api/publishers/{id}/books`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct PublisherBooksParams {
    /// Page number (1-indexed).
    pub page: Option<u32>,
    /// Results per page (max 100).
    pub per_page: Option<u32>,
    /// Sort field: `added_at` (default), `title`, `sort_title`, `updated_at`, `rating`.
    pub sort_by: Option<String>,
    /// Sort direction: asc or desc (default).
    pub sort_order: Option<String>,
    /// Comma-separated list of relations to include: authors, series, tags, files.
    pub include: Option<String>,
}

// ── Request Bodies ──────────────────────────────────────────────

/// Request body for `POST /api/publishers`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreatePublisherRequest {
    /// Publisher name.
    #[validate(length(min = 1, message = "name must not be empty"))]
    pub name: String,
}

/// Request body for `PUT /api/publishers/{id}`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdatePublisherRequest {
    /// Publisher name.
    #[validate(length(min = 1, message = "name must not be empty"))]
    pub name: Option<String>,
}

// ── Response Types ──────────────────────────────────────────────

/// Publisher response.
#[derive(Debug, Serialize, ToSchema)]
pub struct PublisherResponse {
    pub id: Uuid,
    pub name: String,
}

/// Paginated list of publishers.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaginatedPublishers {
    pub items: Vec<PublisherResponse>,
    pub total: u32,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
}

// ── Conversions ─────────────────────────────────────────────────

impl From<Publisher> for PublisherResponse {
    fn from(publisher: Publisher) -> Self {
        Self {
            id: publisher.id,
            name: publisher.name,
        }
    }
}

impl From<PaginatedResult<Publisher>> for PaginatedPublishers {
    fn from(result: PaginatedResult<Publisher>) -> Self {
        Self {
            items: result.items.into_iter().map(Into::into).collect(),
            total: result.total,
            page: result.page,
            per_page: result.per_page,
            total_pages: result.total_pages,
        }
    }
}
