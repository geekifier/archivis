use std::collections::HashSet;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;
use validator::Validate;

use archivis_core::models::Tag;
use archivis_db::{BookFilter, BookRepository, PaginationParams, SortOrder, TagRepository};

use crate::auth::AuthUser;
use crate::books::types::{BookSummary, PaginatedBooks};
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    CreateTagRequest, PaginatedTags, TagBooksParams, TagListParams, TagResponse, UpdateTagRequest,
};

/// GET /api/tags — paginated list of tags.
#[utoipa::path(
    get,
    path = "/api/tags",
    tag = "tags",
    params(TagListParams),
    responses(
        (status = 200, description = "Paginated tag list", body = PaginatedTags),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_tags(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(params): Query<TagListParams>,
) -> Result<Json<PaginatedTags>, ApiError> {
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

    if params.q.is_some() || params.category.is_some() {
        let result = TagRepository::search(
            pool,
            params.q.as_deref(),
            params.category.as_deref(),
            &pagination,
        )
        .await?;
        return Ok(Json(result.into()));
    }

    let result = TagRepository::list(pool, &pagination).await?;
    Ok(Json(result.into()))
}

/// POST /api/tags — create a new tag.
#[utoipa::path(
    post,
    path = "/api/tags",
    tag = "tags",
    request_body = CreateTagRequest,
    responses(
        (status = 201, description = "Tag created", body = TagResponse),
        (status = 400, description = "Validation error"),
        (status = 409, description = "Tag already exists"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn create_tag(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<CreateTagRequest>,
) -> Result<(StatusCode, Json<TagResponse>), ApiError> {
    body.validate()?;

    let tag = if let Some(ref category) = body.category {
        if category.is_empty() {
            Tag::new(&body.name)
        } else {
            Tag::with_category(&body.name, category)
        }
    } else {
        Tag::new(&body.name)
    };

    TagRepository::create(state.db_pool(), &tag).await?;
    Ok((StatusCode::CREATED, Json(tag.into())))
}

/// GET /api/tags/{id} — get tag by ID.
#[utoipa::path(
    get,
    path = "/api/tags/{id}",
    tag = "tags",
    params(("id" = Uuid, Path, description = "Tag ID")),
    responses(
        (status = 200, description = "Tag detail", body = TagResponse),
        (status = 404, description = "Tag not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn get_tag(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<TagResponse>, ApiError> {
    let tag = TagRepository::get_by_id(state.db_pool(), id).await?;
    Ok(Json(tag.into()))
}

/// PUT /api/tags/{id} — update tag.
#[utoipa::path(
    put,
    path = "/api/tags/{id}",
    tag = "tags",
    params(("id" = Uuid, Path, description = "Tag ID")),
    request_body = UpdateTagRequest,
    responses(
        (status = 200, description = "Updated tag", body = TagResponse),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Tag not found"),
        (status = 409, description = "Tag name conflict"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn update_tag(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateTagRequest>,
) -> Result<Json<TagResponse>, ApiError> {
    body.validate()?;

    let pool = state.db_pool();
    let mut tag = TagRepository::get_by_id(pool, id).await?;

    if let Some(name) = body.name {
        if name.is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
        tag.name = name;
    }
    if let Some(category) = body.category {
        tag.category = Some(category).filter(|s| !s.is_empty());
    }

    TagRepository::update(pool, &tag).await?;
    Ok(Json(tag.into()))
}

/// DELETE /api/tags/{id} — delete tag.
#[utoipa::path(
    delete,
    path = "/api/tags/{id}",
    tag = "tags",
    params(("id" = Uuid, Path, description = "Tag ID")),
    responses(
        (status = 204, description = "Tag deleted"),
        (status = 404, description = "Tag not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn delete_tag(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    TagRepository::delete(state.db_pool(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/tags/{id}/books — books with this tag.
#[utoipa::path(
    get,
    path = "/api/tags/{id}/books",
    tag = "tags",
    params(
        ("id" = Uuid, Path, description = "Tag ID"),
        TagBooksParams,
    ),
    responses(
        (status = 200, description = "Books with tag", body = PaginatedBooks),
        (status = 404, description = "Tag not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_tag_books(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Query(params): Query<TagBooksParams>,
) -> Result<Json<PaginatedBooks>, ApiError> {
    let pool = state.db_pool();

    // Verify tag exists
    TagRepository::get_by_id(pool, id).await?;

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
        tags: Some(vec![id.to_string()]),
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
