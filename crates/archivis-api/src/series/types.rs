use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use archivis_core::models::Series;
use archivis_db::PaginatedResult;

// ── Query Parameters ────────────────────────────────────────────

/// Query parameters for `GET /api/series`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct SeriesListParams {
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

/// Query parameters for `GET /api/series/{id}/books`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct SeriesBooksParams {
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

/// Request body for `POST /api/series`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateSeriesRequest {
    /// Series name.
    #[validate(length(min = 1, message = "name must not be empty"))]
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
}

/// Request body for `PUT /api/series/{id}`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateSeriesRequest {
    /// Series name.
    #[validate(length(min = 1, message = "name must not be empty"))]
    pub name: Option<String>,
    /// Description.
    pub description: Option<String>,
}

// ── Response Types ──────────────────────────────────────────────

/// Series response.
#[derive(Debug, Serialize, ToSchema)]
pub struct SeriesResponse {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
}

/// Paginated list of series.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaginatedSeries {
    pub items: Vec<SeriesResponse>,
    pub total: u32,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
}

// ── Conversions ─────────────────────────────────────────────────

impl From<Series> for SeriesResponse {
    fn from(series: Series) -> Self {
        Self {
            id: series.id,
            name: series.name,
            description: series.description,
        }
    }
}

impl From<PaginatedResult<Series>> for PaginatedSeries {
    fn from(result: PaginatedResult<Series>) -> Self {
        Self {
            items: result.items.into_iter().map(Into::into).collect(),
            total: result.total,
            page: result.page,
            per_page: result.per_page,
            total_pages: result.total_pages,
        }
    }
}
