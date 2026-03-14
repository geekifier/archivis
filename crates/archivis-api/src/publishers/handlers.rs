use std::collections::HashSet;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;
use validator::Validate;

use archivis_core::models::Publisher;
use archivis_db::{BookFilter, BookRepository, PaginationParams, PublisherRepository, SortOrder};

use crate::auth::AuthUser;
use crate::books::types::{BookSummary, PaginatedBooks};
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    CreatePublisherRequest, PaginatedPublishers, PublisherBooksParams, PublisherListParams,
    PublisherResponse, UpdatePublisherRequest,
};

/// GET /api/publishers — paginated list of publishers.
#[utoipa::path(
    get,
    path = "/api/publishers",
    tag = "publishers",
    params(PublisherListParams),
    responses(
        (status = 200, description = "Paginated publisher list", body = PaginatedPublishers),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_publishers(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(params): Query<PublisherListParams>,
) -> Result<Json<PaginatedPublishers>, ApiError> {
    let per_page = params.per_page.unwrap_or(25).min(100);
    let page = params.page.unwrap_or(1).max(1);

    let sort_order = match params.sort_order.as_deref() {
        Some("desc") => SortOrder::Desc,
        _ => SortOrder::Asc,
    };

    let pagination = PaginationParams {
        page,
        per_page,
        sort_by: params.sort_by.unwrap_or_else(|| "name".into()),
        sort_order,
    };

    let pool = state.db_pool();

    if let Some(ref q) = params.q {
        if !q.is_empty() {
            let result = PublisherRepository::search(pool, q, &pagination).await?;
            return Ok(Json(result.into()));
        }
    }

    let result = PublisherRepository::list(pool, &pagination).await?;
    Ok(Json(result.into()))
}

/// POST /api/publishers — create a new publisher.
#[utoipa::path(
    post,
    path = "/api/publishers",
    tag = "publishers",
    request_body = CreatePublisherRequest,
    responses(
        (status = 201, description = "Publisher created", body = PublisherResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn create_publisher(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<CreatePublisherRequest>,
) -> Result<(StatusCode, Json<PublisherResponse>), ApiError> {
    body.validate()?;

    let publisher = Publisher::new(&body.name);

    PublisherRepository::create(state.db_pool(), &publisher).await?;
    Ok((StatusCode::CREATED, Json(publisher.into())))
}

/// GET /api/publishers/{id} — get publisher by ID.
#[utoipa::path(
    get,
    path = "/api/publishers/{id}",
    tag = "publishers",
    params(("id" = Uuid, Path, description = "Publisher ID")),
    responses(
        (status = 200, description = "Publisher detail", body = PublisherResponse),
        (status = 404, description = "Publisher not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn get_publisher(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<PublisherResponse>, ApiError> {
    let publisher = PublisherRepository::get_by_id(state.db_pool(), id).await?;
    Ok(Json(publisher.into()))
}

/// PUT /api/publishers/{id} — update publisher.
#[utoipa::path(
    put,
    path = "/api/publishers/{id}",
    tag = "publishers",
    params(("id" = Uuid, Path, description = "Publisher ID")),
    request_body = UpdatePublisherRequest,
    responses(
        (status = 200, description = "Updated publisher", body = PublisherResponse),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Publisher not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn update_publisher(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdatePublisherRequest>,
) -> Result<Json<PublisherResponse>, ApiError> {
    body.validate()?;

    let pool = state.db_pool();
    let mut publisher = PublisherRepository::get_by_id(pool, id).await?;

    if let Some(name) = body.name {
        if name.is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
        publisher.name = name;
    }

    PublisherRepository::update(pool, &publisher).await?;
    Ok(Json(publisher.into()))
}

/// DELETE /api/publishers/{id} — delete publisher (rejected if books still reference it).
#[utoipa::path(
    delete,
    path = "/api/publishers/{id}",
    tag = "publishers",
    params(("id" = Uuid, Path, description = "Publisher ID")),
    responses(
        (status = 204, description = "Publisher deleted"),
        (status = 404, description = "Publisher not found"),
        (status = 409, description = "Publisher still referenced by books"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn delete_publisher(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db_pool();

    // Verify publisher exists
    PublisherRepository::get_by_id(pool, id).await?;

    // Reject if books still reference this publisher
    let book_count = PublisherRepository::count_books(pool, id).await?;
    if book_count > 0 {
        return Err(ApiError::Validation(format!(
            "cannot delete publisher: {book_count} book(s) still reference it"
        )));
    }

    PublisherRepository::delete(pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/publishers/{id}/books — books by this publisher.
#[utoipa::path(
    get,
    path = "/api/publishers/{id}/books",
    tag = "publishers",
    params(
        ("id" = Uuid, Path, description = "Publisher ID"),
        PublisherBooksParams,
    ),
    responses(
        (status = 200, description = "Books by publisher", body = PaginatedBooks),
        (status = 404, description = "Publisher not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_publisher_books(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Query(params): Query<PublisherBooksParams>,
) -> Result<Json<PaginatedBooks>, ApiError> {
    let pool = state.db_pool();

    // Verify publisher exists
    PublisherRepository::get_by_id(pool, id).await?;

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
        publisher_id: Some(id.to_string()),
        trusted: None,
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
