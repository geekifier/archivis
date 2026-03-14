use std::collections::HashSet;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;
use validator::Validate;

use archivis_core::models::Series;
use archivis_db::{BookFilter, BookRepository, PaginationParams, SeriesRepository, SortOrder};

use crate::auth::AuthUser;
use crate::books::types::{BookSummary, PaginatedBooks};
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    CreateSeriesRequest, PaginatedSeries, SeriesBooksParams, SeriesListParams, SeriesResponse,
    UpdateSeriesRequest,
};

/// GET /api/series — paginated list of series.
#[utoipa::path(
    get,
    path = "/api/series",
    tag = "series",
    params(SeriesListParams),
    responses(
        (status = 200, description = "Paginated series list", body = PaginatedSeries),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_series(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(params): Query<SeriesListParams>,
) -> Result<Json<PaginatedSeries>, ApiError> {
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
            let result = SeriesRepository::search(pool, q, &pagination).await?;
            return Ok(Json(result.into()));
        }
    }

    let result = SeriesRepository::list(pool, &pagination).await?;
    Ok(Json(result.into()))
}

/// POST /api/series — create a new series.
#[utoipa::path(
    post,
    path = "/api/series",
    tag = "series",
    request_body = CreateSeriesRequest,
    responses(
        (status = 201, description = "Series created", body = SeriesResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn create_series(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<CreateSeriesRequest>,
) -> Result<(StatusCode, Json<SeriesResponse>), ApiError> {
    body.validate()?;

    let mut series = Series::new(&body.name);
    series.description = body.description.filter(|s| !s.is_empty());

    SeriesRepository::create(state.db_pool(), &series).await?;
    Ok((StatusCode::CREATED, Json(series.into())))
}

/// GET /api/series/{id} — get series by ID.
#[utoipa::path(
    get,
    path = "/api/series/{id}",
    tag = "series",
    params(("id" = Uuid, Path, description = "Series ID")),
    responses(
        (status = 200, description = "Series detail", body = SeriesResponse),
        (status = 404, description = "Series not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn get_series(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<SeriesResponse>, ApiError> {
    let series = SeriesRepository::get_by_id(state.db_pool(), id).await?;
    Ok(Json(series.into()))
}

/// PUT /api/series/{id} — update series.
#[utoipa::path(
    put,
    path = "/api/series/{id}",
    tag = "series",
    params(("id" = Uuid, Path, description = "Series ID")),
    request_body = UpdateSeriesRequest,
    responses(
        (status = 200, description = "Updated series", body = SeriesResponse),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Series not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn update_series(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateSeriesRequest>,
) -> Result<Json<SeriesResponse>, ApiError> {
    body.validate()?;

    let pool = state.db_pool();
    let mut series = SeriesRepository::get_by_id(pool, id).await?;

    if let Some(name) = body.name {
        if name.is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
        series.name = name;
    }
    if let Some(description) = body.description {
        series.description = Some(description).filter(|s| !s.is_empty());
    }

    SeriesRepository::update(pool, &series).await?;
    Ok(Json(series.into()))
}

/// DELETE /api/series/{id} — delete series.
#[utoipa::path(
    delete,
    path = "/api/series/{id}",
    tag = "series",
    params(("id" = Uuid, Path, description = "Series ID")),
    responses(
        (status = 204, description = "Series deleted"),
        (status = 404, description = "Series not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn delete_series(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    SeriesRepository::delete(state.db_pool(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/series/{id}/books — books in this series.
#[utoipa::path(
    get,
    path = "/api/series/{id}/books",
    tag = "series",
    params(
        ("id" = Uuid, Path, description = "Series ID"),
        SeriesBooksParams,
    ),
    responses(
        (status = 200, description = "Books in series", body = PaginatedBooks),
        (status = 404, description = "Series not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_series_books(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Query(params): Query<SeriesBooksParams>,
) -> Result<Json<PaginatedBooks>, ApiError> {
    let pool = state.db_pool();

    // Verify series exists
    SeriesRepository::get_by_id(pool, id).await?;

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
        series_id: Some(id.to_string()),
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
