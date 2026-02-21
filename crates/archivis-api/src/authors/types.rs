use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use archivis_core::models::Author;
use archivis_db::PaginatedResult;

// ── Query Parameters ────────────────────────────────────────────

/// Query parameters for `GET /api/authors`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct AuthorListParams {
    /// Page number (1-indexed).
    pub page: Option<u32>,
    /// Results per page (max 100).
    pub per_page: Option<u32>,
    /// Sort field: `sort_name` (default), `name`.
    pub sort_by: Option<String>,
    /// Sort direction: asc (default) or desc.
    pub sort_order: Option<String>,
    /// Search by name (case-insensitive substring match).
    pub q: Option<String>,
}

/// Query parameters for `GET /api/authors/{id}/books`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct AuthorBooksParams {
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

/// Request body for `POST /api/authors`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateAuthorRequest {
    /// Author's display name.
    #[validate(length(min = 1, message = "name must not be empty"))]
    pub name: String,
    /// Optional sort name override. Auto-generated from name if omitted.
    pub sort_name: Option<String>,
}

/// Request body for `PUT /api/authors/{id}`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateAuthorRequest {
    /// Author's display name.
    #[validate(length(min = 1, message = "name must not be empty"))]
    pub name: Option<String>,
    /// Sort name override.
    pub sort_name: Option<String>,
}

// ── Response Types ──────────────────────────────────────────────

/// Author response.
#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorResponse {
    pub id: Uuid,
    pub name: String,
    pub sort_name: String,
}

/// Paginated list of authors.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaginatedAuthors {
    pub items: Vec<AuthorResponse>,
    pub total: u32,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
}

// ── Conversions ─────────────────────────────────────────────────

impl From<Author> for AuthorResponse {
    fn from(author: Author) -> Self {
        Self {
            id: author.id,
            name: author.name,
            sort_name: author.sort_name,
        }
    }
}

impl From<PaginatedResult<Author>> for PaginatedAuthors {
    fn from(result: PaginatedResult<Author>) -> Self {
        Self {
            items: result.items.into_iter().map(Into::into).collect(),
            total: result.total,
            page: result.page,
            per_page: result.per_page,
            total_pages: result.total_pages,
        }
    }
}
