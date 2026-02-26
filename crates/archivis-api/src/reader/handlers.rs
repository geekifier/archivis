use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use archivis_core::models::Bookmark;
use archivis_db::{
    BookFileRepository, BookRepository, BookmarkRepository, ReadingProgressRepository,
};

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    BookmarkResponse, ContinueReadingItem, CreateBookmarkRequest, ReadingProgressResponse,
    UpdateProgressRequest,
};

/// `GET /api/reader/progress/{book_id}` — get reading progress for a book.
#[utoipa::path(
    get,
    path = "/api/reader/progress/{book_id}",
    tag = "reader",
    params(
        ("book_id" = Uuid, Path, description = "Book ID"),
    ),
    responses(
        (status = 200, description = "Reading progress", body = ReadingProgressResponse),
        (status = 404, description = "No progress found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn get_progress(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(book_id): Path<Uuid>,
) -> Result<Json<ReadingProgressResponse>, ApiError> {
    let pool = state.db_pool();
    let progress = ReadingProgressRepository::get_for_book(pool, user.id, book_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("no reading progress found".into()))?;

    Ok(Json(ReadingProgressResponse {
        id: progress.id.to_string(),
        book_id: progress.book_id.to_string(),
        book_file_id: progress.book_file_id.to_string(),
        location: progress.location,
        progress: progress.progress,
        device_id: progress.device_id,
        preferences: progress.preferences,
        started_at: progress.started_at.to_rfc3339(),
        updated_at: progress.updated_at.to_rfc3339(),
    }))
}

/// `PUT /api/reader/progress/{book_id}/{file_id}` — update reading progress.
#[utoipa::path(
    put,
    path = "/api/reader/progress/{book_id}/{file_id}",
    tag = "reader",
    params(
        ("book_id" = Uuid, Path, description = "Book ID"),
        ("file_id" = Uuid, Path, description = "Book file ID"),
    ),
    request_body = UpdateProgressRequest,
    responses(
        (status = 200, description = "Updated reading progress", body = ReadingProgressResponse),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Book file not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn update_progress(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((book_id, file_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<UpdateProgressRequest>,
) -> Result<Json<ReadingProgressResponse>, ApiError> {
    // Validate progress range
    if !(0.0..=1.0).contains(&body.progress) {
        return Err(ApiError::Validation(
            "progress must be between 0.0 and 1.0".into(),
        ));
    }

    let pool = state.db_pool();

    // Verify file belongs to book
    let book_file = BookFileRepository::get_by_id(pool, file_id).await?;
    if book_file.book_id != book_id {
        return Err(ApiError::NotFound("book file not found".into()));
    }

    let progress = ReadingProgressRepository::upsert(
        pool,
        user.id,
        file_id,
        book_id,
        body.location.as_deref(),
        body.progress,
        body.device_id.as_deref(),
        body.preferences.as_ref(),
    )
    .await?;

    Ok(Json(ReadingProgressResponse {
        id: progress.id.to_string(),
        book_id: progress.book_id.to_string(),
        book_file_id: progress.book_file_id.to_string(),
        location: progress.location,
        progress: progress.progress,
        device_id: progress.device_id,
        preferences: progress.preferences,
        started_at: progress.started_at.to_rfc3339(),
        updated_at: progress.updated_at.to_rfc3339(),
    }))
}

/// `DELETE /api/reader/progress/{book_id}` — delete reading progress for a book.
#[utoipa::path(
    delete,
    path = "/api/reader/progress/{book_id}",
    tag = "reader",
    params(
        ("book_id" = Uuid, Path, description = "Book ID"),
    ),
    responses(
        (status = 204, description = "Progress deleted"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn delete_progress(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(book_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db_pool();
    ReadingProgressRepository::delete_for_book(pool, user.id, book_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct ContinueReadingQuery {
    pub limit: Option<i64>,
}

/// `GET /api/reader/continue-reading` — list recently read books.
#[utoipa::path(
    get,
    path = "/api/reader/continue-reading",
    tag = "reader",
    params(
        ("limit" = Option<i64>, Query, description = "Maximum number of items (default 10, max 50)"),
    ),
    responses(
        (status = 200, description = "Continue reading list", body = Vec<ContinueReadingItem>),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn continue_reading(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Query(query): Query<ContinueReadingQuery>,
) -> Result<Json<Vec<ContinueReadingItem>>, ApiError> {
    let limit = query.limit.unwrap_or(10).clamp(1, 50);
    let pool = state.db_pool();

    let progress_list = ReadingProgressRepository::list_recent(pool, user.id, limit).await?;

    let mut items = Vec::with_capacity(progress_list.len());
    for progress in progress_list {
        // Join book data for each progress record
        let book = BookRepository::get_by_id(pool, progress.book_id).await?;
        let book_file = BookFileRepository::get_by_id(pool, progress.book_file_id).await?;

        items.push(ContinueReadingItem {
            book_id: progress.book_id.to_string(),
            book_title: book.title,
            book_file_id: progress.book_file_id.to_string(),
            file_format: book_file.format.extension().to_string(),
            progress: progress.progress,
            location: progress.location,
            has_cover: book.cover_path.is_some(),
            updated_at: progress.updated_at.to_rfc3339(),
        });
    }

    Ok(Json(items))
}

/// `GET /api/reader/bookmarks/{book_id}/{file_id}` — list bookmarks for a file.
#[utoipa::path(
    get,
    path = "/api/reader/bookmarks/{book_id}/{file_id}",
    tag = "reader",
    params(
        ("book_id" = Uuid, Path, description = "Book ID"),
        ("file_id" = Uuid, Path, description = "Book file ID"),
    ),
    responses(
        (status = 200, description = "Bookmark list", body = Vec<BookmarkResponse>),
        (status = 404, description = "Book file not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_bookmarks(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((book_id, file_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<BookmarkResponse>>, ApiError> {
    let pool = state.db_pool();

    // Verify file belongs to book
    let book_file = BookFileRepository::get_by_id(pool, file_id).await?;
    if book_file.book_id != book_id {
        return Err(ApiError::NotFound("book file not found".into()));
    }

    let bookmarks = BookmarkRepository::list_for_file(pool, user.id, file_id).await?;

    let items: Vec<BookmarkResponse> = bookmarks
        .into_iter()
        .map(|b| BookmarkResponse {
            id: b.id.to_string(),
            location: b.location,
            label: b.label,
            excerpt: b.excerpt,
            position: b.position,
            created_at: b.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(items))
}

/// `POST /api/reader/bookmarks/{book_id}/{file_id}` — create a bookmark.
#[utoipa::path(
    post,
    path = "/api/reader/bookmarks/{book_id}/{file_id}",
    tag = "reader",
    params(
        ("book_id" = Uuid, Path, description = "Book ID"),
        ("file_id" = Uuid, Path, description = "Book file ID"),
    ),
    request_body = CreateBookmarkRequest,
    responses(
        (status = 201, description = "Bookmark created", body = BookmarkResponse),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Book file not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn create_bookmark(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((book_id, file_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<CreateBookmarkRequest>,
) -> Result<(StatusCode, Json<BookmarkResponse>), ApiError> {
    // Validate position range
    if !(0.0..=1.0).contains(&body.position) {
        return Err(ApiError::Validation(
            "position must be between 0.0 and 1.0".into(),
        ));
    }

    let pool = state.db_pool();

    // Verify file belongs to book
    let book_file = BookFileRepository::get_by_id(pool, file_id).await?;
    if book_file.book_id != book_id {
        return Err(ApiError::NotFound("book file not found".into()));
    }

    let bookmark = Bookmark {
        id: Uuid::new_v4(),
        user_id: user.id,
        book_id,
        book_file_id: file_id,
        location: body.location,
        label: body.label,
        excerpt: body.excerpt,
        position: body.position,
        created_at: Utc::now(),
    };

    BookmarkRepository::create(pool, &bookmark).await?;

    Ok((
        StatusCode::CREATED,
        Json(BookmarkResponse {
            id: bookmark.id.to_string(),
            location: bookmark.location,
            label: bookmark.label,
            excerpt: bookmark.excerpt,
            position: bookmark.position,
            created_at: bookmark.created_at.to_rfc3339(),
        }),
    ))
}

/// Request body for updating a bookmark (label only for now).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateBookmarkRequest {
    pub label: Option<String>,
}

/// `PUT /api/reader/bookmarks/{bookmark_id}` — update a bookmark's label.
#[utoipa::path(
    put,
    path = "/api/reader/bookmarks/{bookmark_id}",
    tag = "reader",
    params(
        ("bookmark_id" = Uuid, Path, description = "Bookmark ID"),
    ),
    request_body = UpdateBookmarkRequest,
    responses(
        (status = 200, description = "Updated bookmark", body = BookmarkResponse),
        (status = 404, description = "Bookmark not found or not owned by user"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn update_bookmark(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(bookmark_id): Path<Uuid>,
    Json(body): Json<UpdateBookmarkRequest>,
) -> Result<Json<BookmarkResponse>, ApiError> {
    let pool = state.db_pool();

    // Update label with ownership check
    BookmarkRepository::update_label(pool, bookmark_id, user.id, body.label.as_deref()).await?;

    // Re-fetch the updated bookmark
    let bookmark = BookmarkRepository::get_by_id(pool, bookmark_id).await?;

    Ok(Json(BookmarkResponse {
        id: bookmark.id.to_string(),
        location: bookmark.location,
        label: bookmark.label,
        excerpt: bookmark.excerpt,
        position: bookmark.position,
        created_at: bookmark.created_at.to_rfc3339(),
    }))
}

/// `DELETE /api/reader/bookmarks/{bookmark_id}` — delete a bookmark.
#[utoipa::path(
    delete,
    path = "/api/reader/bookmarks/{bookmark_id}",
    tag = "reader",
    params(
        ("bookmark_id" = Uuid, Path, description = "Bookmark ID"),
    ),
    responses(
        (status = 204, description = "Bookmark deleted"),
        (status = 404, description = "Bookmark not found or not owned by user"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn delete_bookmark(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(bookmark_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db_pool();
    BookmarkRepository::delete(pool, bookmark_id, user.id).await?;
    Ok(StatusCode::NO_CONTENT)
}
