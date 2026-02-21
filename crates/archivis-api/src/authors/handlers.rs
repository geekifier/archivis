use std::collections::HashSet;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;
use validator::Validate;

use archivis_core::models::Author;
use archivis_db::{AuthorRepository, BookFilter, BookRepository, PaginationParams, SortOrder};

use crate::auth::AuthUser;
use crate::books::types::{BookSummary, PaginatedBooks};
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    AuthorBooksParams, AuthorListParams, AuthorResponse, CreateAuthorRequest, PaginatedAuthors,
    UpdateAuthorRequest,
};

/// GET /api/authors — paginated list of authors.
#[utoipa::path(
    get,
    path = "/api/authors",
    tag = "authors",
    params(AuthorListParams),
    responses(
        (status = 200, description = "Paginated author list", body = PaginatedAuthors),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_authors(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(params): Query<AuthorListParams>,
) -> Result<Json<PaginatedAuthors>, ApiError> {
    let per_page = params.per_page.unwrap_or(25).min(100);
    let page = params.page.unwrap_or(1).max(1);

    let sort_order = match params.sort_order.as_deref() {
        Some("desc") => SortOrder::Desc,
        _ => SortOrder::Asc,
    };

    let pagination = PaginationParams {
        page,
        per_page,
        sort_by: params.sort_by.unwrap_or_else(|| "sort_name".into()),
        sort_order,
    };

    let pool = state.db_pool();

    if let Some(ref q) = params.q {
        if !q.is_empty() {
            let result = AuthorRepository::search(pool, q, &pagination).await?;
            return Ok(Json(result.into()));
        }
    }

    let result = AuthorRepository::list(pool, &pagination).await?;
    Ok(Json(result.into()))
}

/// POST /api/authors — create a new author.
#[utoipa::path(
    post,
    path = "/api/authors",
    tag = "authors",
    request_body = CreateAuthorRequest,
    responses(
        (status = 201, description = "Author created", body = AuthorResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn create_author(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<CreateAuthorRequest>,
) -> Result<(StatusCode, Json<AuthorResponse>), ApiError> {
    body.validate()?;

    let mut author = Author::new(&body.name);
    if let Some(sort_name) = body.sort_name {
        if sort_name.is_empty() {
            return Err(ApiError::Validation("sort_name must not be empty".into()));
        }
        author.sort_name = sort_name;
    }

    AuthorRepository::create(state.db_pool(), &author).await?;
    Ok((StatusCode::CREATED, Json(author.into())))
}

/// GET /api/authors/{id} — get author by ID.
#[utoipa::path(
    get,
    path = "/api/authors/{id}",
    tag = "authors",
    params(("id" = Uuid, Path, description = "Author ID")),
    responses(
        (status = 200, description = "Author detail", body = AuthorResponse),
        (status = 404, description = "Author not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn get_author(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<AuthorResponse>, ApiError> {
    let author = AuthorRepository::get_by_id(state.db_pool(), id).await?;
    Ok(Json(author.into()))
}

/// PUT /api/authors/{id} — update author.
#[utoipa::path(
    put,
    path = "/api/authors/{id}",
    tag = "authors",
    params(("id" = Uuid, Path, description = "Author ID")),
    request_body = UpdateAuthorRequest,
    responses(
        (status = 200, description = "Updated author", body = AuthorResponse),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Author not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn update_author(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateAuthorRequest>,
) -> Result<Json<AuthorResponse>, ApiError> {
    body.validate()?;

    let pool = state.db_pool();
    let mut author = AuthorRepository::get_by_id(pool, id).await?;

    if let Some(name) = body.name {
        if name.is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
        // Re-generate sort_name from new name unless sort_name is also provided
        if body.sort_name.is_none() {
            let new_author = Author::new(&name);
            author.sort_name = new_author.sort_name;
        }
        author.name = name;
    }
    if let Some(sort_name) = body.sort_name {
        if sort_name.is_empty() {
            return Err(ApiError::Validation("sort_name must not be empty".into()));
        }
        author.sort_name = sort_name;
    }

    AuthorRepository::update(pool, &author).await?;
    Ok(Json(author.into()))
}

/// DELETE /api/authors/{id} — delete author.
#[utoipa::path(
    delete,
    path = "/api/authors/{id}",
    tag = "authors",
    params(("id" = Uuid, Path, description = "Author ID")),
    responses(
        (status = 204, description = "Author deleted"),
        (status = 404, description = "Author not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn delete_author(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    AuthorRepository::delete(state.db_pool(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/authors/{id}/books — books by this author.
#[utoipa::path(
    get,
    path = "/api/authors/{id}/books",
    tag = "authors",
    params(
        ("id" = Uuid, Path, description = "Author ID"),
        AuthorBooksParams,
    ),
    responses(
        (status = 200, description = "Books by author", body = PaginatedBooks),
        (status = 404, description = "Author not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_author_books(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Query(params): Query<AuthorBooksParams>,
) -> Result<Json<PaginatedBooks>, ApiError> {
    let pool = state.db_pool();

    // Verify author exists
    AuthorRepository::get_by_id(pool, id).await?;

    let per_page = params.per_page.unwrap_or(25).min(100);
    let page = params.page.unwrap_or(1).max(1);

    let sort_order = match params.sort_order.as_deref() {
        Some("asc") => SortOrder::Asc,
        _ => SortOrder::Desc,
    };

    let pagination = PaginationParams {
        page,
        per_page,
        sort_by: params.sort_by.unwrap_or_else(|| "added_at".into()),
        sort_order,
    };

    let filter = BookFilter {
        author_id: Some(id.to_string()),
        ..BookFilter::default()
    };

    let result = BookRepository::list(pool, &pagination, &filter).await?;

    let includes: HashSet<&str> = params
        .include
        .as_deref()
        .map(|s| s.split(',').map(str::trim).collect())
        .unwrap_or_default();

    let mut books: PaginatedBooks = result.into();

    if !includes.is_empty() {
        for summary in &mut books.items {
            enrich_summary(pool, summary, &includes).await?;
        }
    }

    Ok(Json(books))
}

/// Populate optional relation fields on a `BookSummary`.
async fn enrich_summary(
    pool: &archivis_db::DbPool,
    summary: &mut BookSummary,
    includes: &HashSet<&str>,
) -> Result<(), ApiError> {
    let bwr = BookRepository::get_with_relations(pool, summary.id).await?;

    if includes.contains("authors") {
        summary.authors = Some(bwr.authors.into_iter().map(Into::into).collect());
    }
    if includes.contains("series") {
        summary.series = Some(bwr.series.into_iter().map(Into::into).collect());
    }
    if includes.contains("tags") {
        summary.tags = Some(bwr.tags.into_iter().map(Into::into).collect());
    }
    if includes.contains("files") {
        summary.files = Some(bwr.files.into_iter().map(Into::into).collect());
    }

    Ok(())
}
