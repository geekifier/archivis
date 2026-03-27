use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use archivis_core::models::Tag;
use archivis_db::{PaginatedResult, TagWithBookCount};

// ── Query Parameters ────────────────────────────────────────────

/// Query parameters for `GET /api/tags`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct TagListParams {
    /// Page number (1-indexed).
    pub page: Option<u32>,
    /// Results per page (max 100).
    pub per_page: Option<u32>,
    /// Sort field: `name` (default), `category`, `book_count`.
    pub sort_by: Option<String>,
    /// Sort direction: asc (default) or desc.
    pub sort_order: Option<String>,
    /// Search by name (case-insensitive substring match).
    pub q: Option<String>,
    /// Filter by category.
    pub category: Option<String>,
}

/// Query parameters for `GET /api/tags/{id}/books`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct TagBooksParams {
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

/// Request body for `POST /api/tags`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateTagRequest {
    /// Tag name.
    #[validate(length(min = 1, message = "name must not be empty"))]
    pub name: String,
    /// Optional category (e.g., "genre", "mood").
    pub category: Option<String>,
}

/// Request body for `PUT /api/tags/{id}`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateTagRequest {
    /// Tag name.
    #[validate(length(min = 1, message = "name must not be empty"))]
    pub name: Option<String>,
    /// Category.
    pub category: Option<String>,
}

// ── Response Types ──────────────────────────────────────────────

/// Tag response.
#[derive(Debug, Serialize, ToSchema)]
pub struct TagResponse {
    pub id: Uuid,
    pub name: String,
    pub category: Option<String>,
    pub book_count: u32,
}

/// Paginated list of tags.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaginatedTags {
    pub items: Vec<TagResponse>,
    pub total: u32,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
}

// ── Conversions ─────────────────────────────────────────────────

impl From<Tag> for TagResponse {
    fn from(tag: Tag) -> Self {
        Self {
            id: tag.id,
            name: tag.name,
            category: tag.category,
            book_count: 0,
        }
    }
}

impl From<TagWithBookCount> for TagResponse {
    fn from(twc: TagWithBookCount) -> Self {
        Self {
            id: twc.tag.id,
            name: twc.tag.name,
            category: twc.tag.category,
            book_count: twc.book_count,
        }
    }
}

impl From<PaginatedResult<TagWithBookCount>> for PaginatedTags {
    fn from(result: PaginatedResult<TagWithBookCount>) -> Self {
        Self {
            items: result.items.into_iter().map(Into::into).collect(),
            total: result.total,
            page: result.page,
            per_page: result.per_page,
            total_pages: result.total_pages,
        }
    }
}
