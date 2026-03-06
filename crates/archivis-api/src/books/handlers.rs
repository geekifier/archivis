use std::collections::HashSet;

use axum::extract::{Multipart, Path, Query, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_NONE_MATCH,
};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use uuid::Uuid;
use validator::Validate;

use archivis_formats::sanitize::{sanitize_text, SanitizeOptions};
use archivis_formats::CoverData;

use archivis_core::isbn::validate_isbn;
use archivis_core::models::{Book, FieldProvenance, Identifier, IdentifierType, MetadataSource};

use archivis_db::{
    AuthorRepository, BookFileRepository, BookFilter, BookRepository, IdentifierRepository,
    PaginationParams, SeriesRepository, SortOrder, TagRepository,
};
use archivis_storage::StorageBackend;
use archivis_tasks::resolve::persist_recomputed_status;

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    AddIdentifierRequest, BatchBookFields, BatchSetTagsRequest, BatchTagMode, BatchTagsResponse,
    BatchUpdateBooksRequest, BatchUpdateError, BatchUpdateResponse, BookDetail, BookListParams,
    BookSummary, CoverParams, FieldProtectionRequest, OverrideStatusRequest, PaginatedBooks,
    SetBookAuthorsRequest, SetBookSeriesRequest, SetBookTagsRequest, UpdateBookRequest,
    UpdateIdentifierRequest,
};

const USER_EDIT_TRIGGER: &str = "user_edit";
const UNLOCK_TRIGGER: &str = "metadata_unlock";
const PROTECT_FIELDS_TRIGGER: &str = "protect_fields";
const UNPROTECT_FIELDS_TRIGGER: &str = "unprotect_fields";
const PROTECTABLE_FIELDS: &[&str] = &[
    "title",
    "subtitle",
    "description",
    "authors",
    "series",
    "publisher",
    "publication_date",
    "language",
    "page_count",
    "cover",
];

fn user_field_provenance() -> FieldProvenance {
    FieldProvenance {
        origin: MetadataSource::User,
        protected: true,
    }
}

fn validate_protection_fields(fields: &[String]) -> Result<(), ApiError> {
    let mut invalid = Vec::new();

    for field in fields {
        if !PROTECTABLE_FIELDS.contains(&field.as_str()) {
            invalid.push(field.clone());
        }
    }

    if invalid.is_empty() {
        return Ok(());
    }

    invalid.sort();
    invalid.dedup();

    Err(ApiError::Validation(format!(
        "invalid fields values: {}",
        invalid.join(", ")
    )))
}

fn provenance_for_field_mut<'a>(
    book: &'a mut Book,
    field: &str,
) -> Result<&'a mut Option<FieldProvenance>, ApiError> {
    match field {
        "title" => Ok(&mut book.metadata_provenance.title),
        "subtitle" => Ok(&mut book.metadata_provenance.subtitle),
        "description" => Ok(&mut book.metadata_provenance.description),
        "authors" => Ok(&mut book.metadata_provenance.authors),
        "series" => Ok(&mut book.metadata_provenance.series),
        "publisher" => Ok(&mut book.metadata_provenance.publisher),
        "publication_date" => Ok(&mut book.metadata_provenance.publication_date),
        "language" => Ok(&mut book.metadata_provenance.language),
        "page_count" => Ok(&mut book.metadata_provenance.page_count),
        "cover" => Ok(&mut book.metadata_provenance.cover),
        _ => Err(ApiError::Validation(format!("invalid field: {field}"))),
    }
}

fn set_field_protection(
    book: &mut Book,
    fields: &[String],
    protected: bool,
) -> Result<bool, ApiError> {
    validate_protection_fields(fields)?;

    let mut changed = false;
    for field in fields {
        let provenance = provenance_for_field_mut(book, field)?;
        match provenance {
            Some(existing) => {
                if existing.protected != protected {
                    existing.protected = protected;
                    changed = true;
                }
            }
            None if protected => {
                *provenance = Some(user_field_provenance());
                changed = true;
            }
            None => {}
        }
    }

    Ok(changed)
}

async fn invalidate_resolution_for_user_edit(
    pool: &archivis_db::DbPool,
    book_id: Uuid,
) -> Result<(), ApiError> {
    BookRepository::mark_resolution_pending(pool, book_id, USER_EDIT_TRIGGER).await?;
    persist_recomputed_status(pool, book_id).await?;
    Ok(())
}

async fn invalidate_resolution_for_action(
    pool: &archivis_db::DbPool,
    book_id: Uuid,
    trigger: &str,
) -> Result<(), ApiError> {
    BookRepository::mark_resolution_pending(pool, book_id, trigger).await?;
    Ok(())
}

/// GET /api/books — paginated list with sorting, filtering, FTS search.
#[utoipa::path(
    get,
    path = "/api/books",
    tag = "books",
    params(BookListParams),
    responses(
        (status = 200, description = "Paginated book list", body = PaginatedBooks),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_books(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(params): Query<BookListParams>,
) -> Result<Json<PaginatedBooks>, ApiError> {
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

    let format = params
        .format
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|e: String| ApiError::Validation(e))?;

    let status = params
        .status
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|e: String| ApiError::Validation(e))?;

    let filter = BookFilter {
        query: params.q,
        format,
        status,
        tags: None,
        author_id: params.author_id.map(|id| id.to_string()),
        series_id: params.series_id.map(|id| id.to_string()),
        publisher_id: None,
    };

    let pool = state.db_pool();
    let result = BookRepository::list(pool, &pagination, &filter).await?;

    // Parse includes
    let includes: HashSet<&str> = params
        .include
        .as_deref()
        .map(|s| s.split(',').map(str::trim).collect())
        .unwrap_or_default();

    let mut books: PaginatedBooks = result.into();

    // Enrich with relations if requested
    if !includes.is_empty() {
        for summary in &mut books.items {
            enrich_summary(pool, summary, &includes).await?;
        }
    }

    Ok(Json(books))
}

/// Populate optional relation fields on a `BookSummary` based on requested includes.
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

/// GET /api/books/{id} — single book with all relations.
#[utoipa::path(
    get,
    path = "/api/books/{id}",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 200, description = "Book detail", body = BookDetail),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn get_book(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<BookDetail>, ApiError> {
    let bwr = BookRepository::get_with_relations(state.db_pool(), id).await?;
    Ok(Json(bwr.into()))
}

/// PUT /api/books/{id} — update book metadata (partial update).
#[utoipa::path(
    put,
    path = "/api/books/{id}",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = UpdateBookRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn update_book(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateBookRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    body.validate()?;

    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;
    let mut book_changed = false;
    let mut core_identity_changed = false;

    // Always strip dangerous content from user-submitted text
    let sanitize_opts = SanitizeOptions::default();

    if let Some(ref title) = body.title {
        let clean = sanitize_text(title, &sanitize_opts).unwrap_or_default();
        if clean.is_empty() {
            return Err(ApiError::Validation("title must not be empty".into()));
        }
        if clean != book.title {
            book.set_title(clean);
            book.metadata_provenance.title = Some(user_field_provenance());
            book_changed = true;
            core_identity_changed = true;
        }
    }
    if let Some(ref subtitle) = body.subtitle {
        let new_subtitle = if subtitle.is_empty() {
            None
        } else {
            sanitize_text(subtitle, &sanitize_opts)
        };
        if new_subtitle != book.subtitle {
            book.subtitle = new_subtitle;
            book.metadata_provenance.subtitle = Some(user_field_provenance());
            book_changed = true;
        }
    }
    if let Some(ref description) = body.description {
        let new_description = sanitize_text(description, &sanitize_opts);
        if new_description != book.description {
            book.description = new_description;
            book.metadata_provenance.description = Some(user_field_provenance());
            book_changed = true;
        }
    }
    if let Some(language) = body.language {
        let new_language = Some(language).filter(|s| !s.is_empty());
        if new_language != book.language {
            book.language = new_language;
            book.metadata_provenance.language = Some(user_field_provenance());
            book_changed = true;
        }
    }
    if let Some(pub_date) = body.publication_date {
        let new_publication_date = Some(pub_date);
        if new_publication_date != book.publication_date {
            book.publication_date = new_publication_date;
            book.metadata_provenance.publication_date = Some(user_field_provenance());
            book_changed = true;
        }
    }
    if let Some(rating) = body.rating {
        let new_rating = Some(rating);
        if new_rating != book.rating {
            book.rating = new_rating;
            book_changed = true;
        }
    }
    if let Some(page_count) = body.page_count {
        let new_page_count = Some(page_count);
        if new_page_count != book.page_count {
            book.page_count = new_page_count;
            book.metadata_provenance.page_count = Some(user_field_provenance());
            book_changed = true;
        }
    }
    // publisher_id: Some(Some(id)) = set, Some(None) = clear, None = no change
    if let Some(pub_id) = body.publisher_id {
        if pub_id != book.publisher_id {
            book.publisher_id = pub_id;
            book.metadata_provenance.publisher = Some(user_field_provenance());
            book_changed = true;
        }
    }
    if book_changed {
        BookRepository::update(pool, &book).await?;
    }
    if core_identity_changed {
        invalidate_resolution_for_user_edit(pool, id).await?;
    }

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/lock — disable automated metadata mutation for a book.
#[utoipa::path(
    post,
    path = "/api/books/{id}/lock",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn lock_metadata(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    if !book.metadata_locked {
        book.metadata_locked = true;
        BookRepository::update(pool, &book).await?;
    }

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/unlock — re-enable automated metadata mutation for a book.
#[utoipa::path(
    post,
    path = "/api/books/{id}/unlock",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn unlock_metadata(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    if book.metadata_locked {
        book.metadata_locked = false;
        BookRepository::update(pool, &book).await?;
        invalidate_resolution_for_action(pool, id, UNLOCK_TRIGGER).await?;
    }

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/protect-fields — mark fields as protected from automation.
#[utoipa::path(
    post,
    path = "/api/books/{id}/protect-fields",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = FieldProtectionRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn protect_fields(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<FieldProtectionRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    if set_field_protection(&mut book, &body.fields, true)? {
        BookRepository::update(pool, &book).await?;
        invalidate_resolution_for_action(pool, id, PROTECT_FIELDS_TRIGGER).await?;
    }

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/unprotect-fields — clear field protection without changing values.
#[utoipa::path(
    post,
    path = "/api/books/{id}/unprotect-fields",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = FieldProtectionRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn unprotect_fields(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<FieldProtectionRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    if set_field_protection(&mut book, &body.fields, false)? {
        BookRepository::update(pool, &book).await?;
        invalidate_resolution_for_action(pool, id, UNPROTECT_FIELDS_TRIGGER).await?;
    }

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/override-status — manually set metadata status and lock the book.
#[utoipa::path(
    post,
    path = "/api/books/{id}/override-status",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = OverrideStatusRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn override_status(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<OverrideStatusRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    book.metadata_status = body.metadata_status;
    book.metadata_locked = true;
    BookRepository::update(pool, &book).await?;

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// DELETE /api/books/{id} — delete book, its files, and cover.
#[utoipa::path(
    delete,
    path = "/api/books/{id}",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 204, description = "Book deleted"),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn delete_book(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db_pool();
    let bwr = BookRepository::get_with_relations(pool, id).await?;
    let storage = state.storage();

    // Delete book files from storage (ignore not-found)
    for file in &bwr.files {
        if let Err(e) = storage.delete(&file.storage_path).await {
            tracing::warn!(path = %file.storage_path, error = %e, "failed to delete book file from storage");
        }
    }

    // Delete cover from storage
    if let Some(ref cover_path) = bwr.book.cover_path {
        if let Err(e) = storage.delete(cover_path).await {
            tracing::warn!(path = %cover_path, error = %e, "failed to delete cover from storage");
        }
    }

    // Remove thumbnail cache directory
    let cache_dir = state.config().data_dir.join("covers").join(id.to_string());
    if cache_dir.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&cache_dir).await {
            tracing::warn!(path = ?cache_dir, error = %e, "failed to remove thumbnail cache");
        }
    }

    BookRepository::delete(pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/books/{id}/cover — serve cover image.
#[utoipa::path(
    get,
    path = "/api/books/{id}/cover",
    tag = "books",
    params(
        ("id" = Uuid, Path, description = "Book ID"),
        CoverParams,
    ),
    responses(
        (status = 200, description = "Cover image", content_type = "image/*"),
        (status = 304, description = "Not modified"),
        (status = 404, description = "Cover not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn get_cover(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Query(params): Query<CoverParams>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let pool = state.db_pool();
    let book = BookRepository::get_by_id(pool, id).await?;

    let cover_path = book
        .cover_path
        .as_deref()
        .ok_or_else(|| ApiError::NotFound("book has no cover".into()))?;

    let size = params.size.as_deref().unwrap_or("original");
    let data_dir = &state.config().data_dir;

    match size {
        "sm" | "md" => {
            let thumb_path = data_dir
                .join("covers")
                .join(id.to_string())
                .join(format!("{size}.webp"));

            // Lazily generate if missing
            if !thumb_path.exists() {
                let storage = state.storage();
                let cover_bytes = storage.read(cover_path).await?;

                let target_height = if size == "sm" { 150 } else { 300 };

                // Write source to a temp file for generate_thumbnail
                let tmp_dir = tempfile::tempdir()
                    .map_err(|e| ApiError::Internal(format!("failed to create temp dir: {e}")))?;
                let tmp_source = tmp_dir.path().join("source");
                tokio::fs::write(&tmp_source, &cover_bytes)
                    .await
                    .map_err(|e| ApiError::Internal(format!("failed to write temp cover: {e}")))?;

                archivis_tasks::import::generate_thumbnail(
                    &tmp_source,
                    id,
                    data_dir,
                    size,
                    target_height,
                )
                .await
                .map_err(|e| ApiError::Internal(format!("thumbnail generation failed: {e}")))?;
            }

            serve_file_with_etag(&thumb_path, "image/webp", &headers).await
        }
        "lg" => {
            let thumb_path = data_dir.join("covers").join(id.to_string()).join("lg.webp");

            // Lazily generate if missing
            if !thumb_path.exists() {
                let storage = state.storage();
                let cover_bytes = storage.read(cover_path).await?;

                // Write source to a temp file for generate_thumbnail
                let tmp_dir = tempfile::tempdir()
                    .map_err(|e| ApiError::Internal(format!("failed to create temp dir: {e}")))?;
                let tmp_source = tmp_dir.path().join("source");
                tokio::fs::write(&tmp_source, &cover_bytes)
                    .await
                    .map_err(|e| ApiError::Internal(format!("failed to write temp cover: {e}")))?;

                archivis_tasks::import::generate_thumbnail(&tmp_source, id, data_dir, "lg", 600)
                    .await
                    .map_err(|e| ApiError::Internal(format!("thumbnail generation failed: {e}")))?;
            }

            serve_file_with_etag(&thumb_path, "image/webp", &headers).await
        }
        _ => {
            let storage = state.storage();
            let cover_bytes = storage.read(cover_path).await?;

            let content_type = match cover_path.rsplit('.').next() {
                Some("jpg" | "jpeg") => "image/jpeg",
                Some("png") => "image/png",
                Some("gif") => "image/gif",
                Some("webp") => "image/webp",
                Some("svg") => "image/svg+xml",
                _ => "application/octet-stream",
            };

            // Use a hash-based ETag for storage-backed files
            let etag = format!("W/\"{}\"", simple_hash(&cover_bytes));

            if let Some(if_none_match) = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
                if if_none_match == etag {
                    return Ok(StatusCode::NOT_MODIFIED.into_response());
                }
            }

            Ok((
                [
                    (CONTENT_TYPE, content_type.to_string()),
                    (CONTENT_LENGTH, cover_bytes.len().to_string()),
                    (ETAG, etag),
                    (
                        CACHE_CONTROL,
                        "public, max-age=86400, must-revalidate".into(),
                    ),
                ],
                cover_bytes,
            )
                .into_response())
        }
    }
}

/// POST /api/books/{id}/cover — upload or replace cover image.
#[utoipa::path(
    post,
    path = "/api/books/{id}/cover",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 200, description = "Cover uploaded, book updated", body = BookDetail),
        (status = 400, description = "Invalid image or no file provided"),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn upload_cover(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let bwr = BookRepository::get_with_relations(pool, id).await?;
    let mut book = bwr.book;
    let storage = state.storage();
    let data_dir = &state.config().data_dir;

    // Extract the first file field from the multipart form
    let field = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::Validation(format!("multipart error: {e}")))?
        .ok_or_else(|| ApiError::Validation("no file provided".into()))?;

    // Validate content type is an image
    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();

    if !content_type.starts_with("image/") {
        return Err(ApiError::Validation(format!(
            "file must be an image, got: {content_type}"
        )));
    }

    let image_bytes = field
        .bytes()
        .await
        .map_err(|e| ApiError::Validation(format!("failed to read upload: {e}")))?;

    if image_bytes.is_empty() {
        return Err(ApiError::Validation("uploaded file is empty".into()));
    }

    // Determine the book's storage directory from its first file
    let book_path_dir = bwr
        .files
        .first()
        .and_then(|f| {
            let p = &f.storage_path;
            p.rfind('/').map(|idx| &p[..idx])
        })
        .ok_or_else(|| {
            ApiError::Validation("book has no files; cannot determine storage directory".into())
        })?
        .to_string();

    // Delete old cover from storage if present
    if let Some(ref old_cover_path) = book.cover_path {
        if let Err(e) = storage.delete(old_cover_path).await {
            tracing::warn!(path = %old_cover_path, error = %e, "failed to delete old cover from storage");
        }
    }

    // Delete old thumbnail cache directory
    let cache_dir = data_dir.join("covers").join(id.to_string());
    if cache_dir.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&cache_dir).await {
            tracing::warn!(path = ?cache_dir, error = %e, "failed to remove old thumbnail cache");
        }
    }

    // Store new cover via StorageBackend
    let cover_data = CoverData {
        bytes: image_bytes.to_vec(),
        media_type: content_type,
    };

    let new_cover_path = archivis_tasks::import::store_cover(storage, &book_path_dir, &cover_data)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to store cover: {e}")))?;

    // Generate sm + md thumbnails
    let thumbnail_sizes = archivis_tasks::import::ThumbnailSizes::default();
    archivis_tasks::import::generate_thumbnails(&cover_data, id, data_dir, &thumbnail_sizes)
        .await
        .map_err(|e| ApiError::Internal(format!("thumbnail generation failed: {e}")))?;

    // Update book.cover_path in the database
    book.cover_path = Some(new_cover_path);
    BookRepository::update(pool, &book).await?;

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// Serve a local file with ETag/304 support.
async fn serve_file_with_etag(
    path: &std::path::Path,
    content_type: &str,
    req_headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|_| ApiError::NotFound("cover file not found".into()))?;

    let modified = metadata
        .modified()
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let etag = format!(
        "W/\"{}-{}\"",
        modified
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        metadata.len()
    );

    if let Some(if_none_match) = req_headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if if_none_match == etag {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
    }

    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to read cover: {e}")))?;

    Ok((
        [
            (CONTENT_TYPE, content_type.to_string()),
            (CONTENT_LENGTH, bytes.len().to_string()),
            (ETAG, etag),
            (
                CACHE_CONTROL,
                "public, max-age=86400, must-revalidate".into(),
            ),
        ],
        bytes,
    )
        .into_response())
}

/// Simple hash for `ETag` generation.
fn simple_hash(data: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

/// `GET /api/books/{id}/files/{file_id}/download` — stream book file.
#[utoipa::path(
    get,
    path = "/api/books/{id}/files/{file_id}/download",
    tag = "books",
    params(
        ("id" = Uuid, Path, description = "Book ID"),
        ("file_id" = Uuid, Path, description = "Book file ID"),
    ),
    responses(
        (status = 200, description = "File download", content_type = "application/octet-stream"),
        (status = 404, description = "File not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn download_file(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path((book_id, file_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let pool = state.db_pool();
    let book_file = BookFileRepository::get_by_id(pool, file_id).await?;

    // Verify the file belongs to this book
    if book_file.book_id != book_id {
        return Err(ApiError::NotFound("book file not found".into()));
    }

    let storage = state.storage();
    let data = storage.read(&book_file.storage_path).await?;

    let book = BookRepository::get_by_id(pool, book_id).await?;
    let ext = book_file.format.extension();
    let filename = format!("{}.{ext}", book.title);
    // Sanitize filename for Content-Disposition
    let safe_filename = filename.replace('"', "'");

    Ok((
        [
            (CONTENT_TYPE, book_file.format.mime_type().to_string()),
            (
                CONTENT_DISPOSITION,
                format!("attachment; filename=\"{safe_filename}\""),
            ),
            (CONTENT_LENGTH, data.len().to_string()),
        ],
        data,
    )
        .into_response())
}

/// `GET /api/books/{id}/files/{file_id}/content` — serve book file for in-browser reading.
///
/// Unlike /download (Content-Disposition: attachment), this uses inline disposition
/// and aggressive caching since ebook files are immutable after import.
#[utoipa::path(
    get,
    path = "/api/books/{id}/files/{file_id}/content",
    tag = "books",
    params(
        ("id" = Uuid, Path, description = "Book ID"),
        ("file_id" = Uuid, Path, description = "Book file ID"),
    ),
    responses(
        (status = 200, description = "File content", content_type = "application/octet-stream"),
        (status = 304, description = "Not modified"),
        (status = 404, description = "File not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn serve_file_content(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path((book_id, file_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let pool = state.db_pool();
    let book_file = BookFileRepository::get_by_id(pool, file_id).await?;

    // Verify the file belongs to this book
    if book_file.book_id != book_id {
        return Err(ApiError::NotFound("book file not found".into()));
    }

    // ETag based on the file's SHA-256 hash
    let etag = format!("\"{}\"", book_file.hash);

    // Check If-None-Match for conditional request
    if let Some(if_none_match) = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if if_none_match == etag {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
    }

    let storage = state.storage();
    let data = storage.read(&book_file.storage_path).await?;

    Ok((
        [
            (CONTENT_TYPE, book_file.format.mime_type().to_string()),
            (CONTENT_DISPOSITION, "inline".to_string()),
            (CONTENT_LENGTH, data.len().to_string()),
            (ETAG, etag),
            (CACHE_CONTROL, "public, max-age=604800, immutable".into()),
        ],
        data,
    )
        .into_response())
}

/// POST /api/books/{id}/authors — replace book-author links.
#[utoipa::path(
    post,
    path = "/api/books/{id}/authors",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = SetBookAuthorsRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 404, description = "Book or author not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn set_book_authors(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetBookAuthorsRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    // Verify each author exists
    for link in &body.authors {
        AuthorRepository::get_by_id(pool, link.author_id).await?;
    }

    // Replace all author links
    BookRepository::clear_authors(pool, id).await?;
    for link in &body.authors {
        BookRepository::add_author(pool, id, link.author_id, &link.role, link.position).await?;
    }

    book.metadata_provenance.authors = Some(user_field_provenance());
    BookRepository::update(pool, &book).await?;
    invalidate_resolution_for_user_edit(pool, id).await?;

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/series — replace book-series links.
#[utoipa::path(
    post,
    path = "/api/books/{id}/series",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = SetBookSeriesRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 404, description = "Book or series not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn set_book_series(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetBookSeriesRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();
    let mut book = BookRepository::get_by_id(pool, id).await?;

    // Verify each series exists
    for link in &body.series {
        SeriesRepository::get_by_id(pool, link.series_id).await?;
    }

    // Replace all series links
    BookRepository::clear_series(pool, id).await?;
    for link in &body.series {
        BookRepository::add_series(pool, id, link.series_id, link.position).await?;
    }

    book.metadata_provenance.series = Some(user_field_provenance());
    BookRepository::update(pool, &book).await?;
    invalidate_resolution_for_user_edit(pool, id).await?;

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/tags — replace book-tag links.
#[utoipa::path(
    post,
    path = "/api/books/{id}/tags",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = SetBookTagsRequest,
    responses(
        (status = 200, description = "Updated book", body = BookDetail),
        (status = 400, description = "Tag link must have tag_id or name"),
        (status = 404, description = "Book or tag not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn set_book_tags(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetBookTagsRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, id).await?;

    // Resolve each tag
    let mut tag_ids = Vec::with_capacity(body.tags.len());
    for link in &body.tags {
        let tag_id = if let Some(tid) = link.tag_id {
            // Verify tag exists
            TagRepository::get_by_id(pool, tid).await?;
            tid
        } else if let Some(ref name) = link.name {
            let tag = TagRepository::find_or_create(pool, name, link.category.as_deref()).await?;
            tag.id
        } else {
            return Err(ApiError::Validation(
                "each tag must have either tag_id or name".into(),
            ));
        };
        tag_ids.push(tag_id);
    }

    // Replace all tag links
    BookRepository::clear_tags(pool, id).await?;
    for tag_id in tag_ids {
        BookRepository::add_tag(pool, id, tag_id).await?;
    }

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// POST /api/books/{id}/identifiers — add a new identifier to a book.
#[utoipa::path(
    post,
    path = "/api/books/{id}/identifiers",
    tag = "books",
    params(("id" = Uuid, Path, description = "Book ID")),
    request_body = AddIdentifierRequest,
    responses(
        (status = 200, description = "Identifier added, updated book returned", body = BookDetail),
        (status = 400, description = "Validation error (e.g. invalid ISBN checksum)"),
        (status = 404, description = "Book not found"),
        (status = 409, description = "Duplicate identifier"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn add_identifier(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<AddIdentifierRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    body.validate()?;

    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, id).await?;

    let identifier_type = body.identifier_type;
    let mut value = body.value.trim().to_string();

    // For ISBN types: validate checksum and normalize
    if matches!(
        identifier_type,
        IdentifierType::Isbn13 | IdentifierType::Isbn10
    ) {
        let validation = validate_isbn(&value);
        if !validation.valid {
            return Err(ApiError::Validation(validation.message));
        }
        value = validation.normalized;
    }

    // Serialize the identifier type to its DB string form
    let type_str = serde_json::to_value(identifier_type)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();

    // Check for duplicates
    if IdentifierRepository::exists_for_book(pool, id, &type_str, &value).await? {
        return Err(ApiError::Validation(format!(
            "identifier {type_str}:{value} already exists for this book"
        )));
    }

    // Create the identifier with source: User and confidence: 1.0
    let identifier = Identifier::new(id, identifier_type, &value, MetadataSource::User, 1.0);
    IdentifierRepository::create(pool, &identifier).await?;
    invalidate_resolution_for_user_edit(pool, id).await?;

    let bwr = BookRepository::get_with_relations(pool, id).await?;
    Ok(Json(bwr.into()))
}

/// `PUT /api/books/{id}/identifiers/{identifier_id}` — update an existing identifier.
#[utoipa::path(
    put,
    path = "/api/books/{id}/identifiers/{identifier_id}",
    tag = "books",
    params(
        ("id" = Uuid, Path, description = "Book ID"),
        ("identifier_id" = Uuid, Path, description = "Identifier ID"),
    ),
    request_body = UpdateIdentifierRequest,
    responses(
        (status = 200, description = "Identifier updated, updated book returned", body = BookDetail),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Book or identifier not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn update_identifier(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path((book_id, identifier_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<UpdateIdentifierRequest>,
) -> Result<Json<BookDetail>, ApiError> {
    body.validate()?;

    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, book_id).await?;

    // Load identifier and verify it belongs to this book
    let existing = IdentifierRepository::get_by_id(pool, identifier_id).await?;
    if existing.book_id != book_id {
        return Err(ApiError::NotFound(
            "identifier not found for this book".into(),
        ));
    }

    let new_type = body.identifier_type.unwrap_or(existing.identifier_type);
    let new_value = body
        .value
        .map(|v| v.trim().to_string())
        .unwrap_or(existing.value);

    // Validate ISBN if the type is an ISBN type
    let final_value = if matches!(new_type, IdentifierType::Isbn13 | IdentifierType::Isbn10) {
        let validation = validate_isbn(&new_value);
        if !validation.valid {
            return Err(ApiError::Validation(validation.message));
        }
        validation.normalized
    } else {
        new_value
    };

    let type_str = serde_json::to_value(new_type)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();

    IdentifierRepository::update(pool, identifier_id, &final_value, &type_str).await?;
    invalidate_resolution_for_user_edit(pool, book_id).await?;

    let bwr = BookRepository::get_with_relations(pool, book_id).await?;
    Ok(Json(bwr.into()))
}

/// `DELETE /api/books/{id}/identifiers/{identifier_id}` — remove an identifier.
#[utoipa::path(
    delete,
    path = "/api/books/{id}/identifiers/{identifier_id}",
    tag = "books",
    params(
        ("id" = Uuid, Path, description = "Book ID"),
        ("identifier_id" = Uuid, Path, description = "Identifier ID"),
    ),
    responses(
        (status = 204, description = "Identifier deleted"),
        (status = 404, description = "Book or identifier not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn delete_identifier(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path((book_id, identifier_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db_pool();

    // Verify book exists
    BookRepository::get_by_id(pool, book_id).await?;

    // Load identifier and verify it belongs to this book
    let existing = IdentifierRepository::get_by_id(pool, identifier_id).await?;
    if existing.book_id != book_id {
        return Err(ApiError::NotFound(
            "identifier not found for this book".into(),
        ));
    }

    IdentifierRepository::delete(pool, identifier_id).await?;
    invalidate_resolution_for_user_edit(pool, book_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/books/batch-update -- batch update scalar fields on multiple books.
#[utoipa::path(
    post,
    path = "/api/books/batch-update",
    tag = "books",
    request_body = BatchUpdateBooksRequest,
    responses(
        (status = 200, description = "Batch update result", body = BatchUpdateResponse),
        (status = 400, description = "Validation error (e.g. too many IDs)"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn batch_update_books(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<BatchUpdateBooksRequest>,
) -> Result<Json<BatchUpdateResponse>, ApiError> {
    body.validate()?;

    if body.book_ids.is_empty() {
        return Err(ApiError::Validation("book_ids must not be empty".into()));
    }
    if body.book_ids.len() > 100 {
        return Err(ApiError::Validation(
            "batch update supports at most 100 books per request".into(),
        ));
    }

    let pool = state.db_pool();
    let sanitize_opts = SanitizeOptions::default();
    let mut updated_count: u32 = 0;
    let mut errors = Vec::new();

    for &book_id in &body.book_ids {
        match apply_batch_fields(pool, book_id, &body.updates, &sanitize_opts).await {
            Ok(()) => updated_count += 1,
            Err(e) => errors.push(BatchUpdateError {
                book_id,
                error: e.to_string(),
            }),
        }
    }

    Ok(Json(BatchUpdateResponse {
        updated_count,
        errors,
    }))
}

/// Apply batch field updates to a single book. Returns `Ok(())` on success.
async fn apply_batch_fields(
    pool: &archivis_db::DbPool,
    book_id: Uuid,
    fields: &BatchBookFields,
    sanitize_opts: &SanitizeOptions,
) -> Result<(), ApiError> {
    let mut book = BookRepository::get_by_id(pool, book_id).await?;
    let mut book_changed = false;

    if let Some(ref language) = fields.language {
        let clean = sanitize_text(language, sanitize_opts).unwrap_or_default();
        let new_language = Some(clean).filter(|s| !s.is_empty());
        if new_language != book.language {
            book.language = new_language;
            book.metadata_provenance.language = Some(user_field_provenance());
            book_changed = true;
        }
    }
    if let Some(rating) = fields.rating {
        let new_rating = Some(rating);
        if new_rating != book.rating {
            book.rating = new_rating;
            book_changed = true;
        }
    }
    if let Some(ref pub_id) = fields.publisher_id {
        if *pub_id != book.publisher_id {
            book.publisher_id = *pub_id;
            book.metadata_provenance.publisher = Some(user_field_provenance());
            book_changed = true;
        }
    }

    if book_changed {
        BookRepository::update(pool, &book).await?;
    }
    Ok(())
}

/// POST /api/books/batch-tags -- batch set or add tags on multiple books.
#[utoipa::path(
    post,
    path = "/api/books/batch-tags",
    tag = "books",
    request_body = BatchSetTagsRequest,
    responses(
        (status = 200, description = "Batch tag update result", body = BatchTagsResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn batch_set_tags(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<BatchSetTagsRequest>,
) -> Result<Json<BatchTagsResponse>, ApiError> {
    if body.book_ids.is_empty() {
        return Err(ApiError::Validation("book_ids must not be empty".into()));
    }
    if body.book_ids.len() > 100 {
        return Err(ApiError::Validation(
            "batch tag update supports at most 100 books per request".into(),
        ));
    }

    let pool = state.db_pool();

    // Resolve all tag IDs up front (shared across books).
    let mut tag_ids = Vec::with_capacity(body.tags.len());
    for link in &body.tags {
        let tag_id = if let Some(tid) = link.tag_id {
            TagRepository::get_by_id(pool, tid).await?;
            tid
        } else if let Some(ref name) = link.name {
            let tag = TagRepository::find_or_create(pool, name, link.category.as_deref()).await?;
            tag.id
        } else {
            return Err(ApiError::Validation(
                "each tag must have either tag_id or name".into(),
            ));
        };
        tag_ids.push(tag_id);
    }

    let mut updated_count: u32 = 0;
    let mut errors = Vec::new();

    for &book_id in &body.book_ids {
        match apply_batch_tags(pool, book_id, &tag_ids, &body.mode).await {
            Ok(()) => updated_count += 1,
            Err(e) => errors.push(BatchUpdateError {
                book_id,
                error: e.to_string(),
            }),
        }
    }

    Ok(Json(BatchTagsResponse {
        updated_count,
        errors,
    }))
}

/// Apply tag changes to a single book. Returns `Ok(())` on success.
async fn apply_batch_tags(
    pool: &archivis_db::DbPool,
    book_id: Uuid,
    tag_ids: &[Uuid],
    mode: &BatchTagMode,
) -> Result<(), ApiError> {
    // Verify book exists
    BookRepository::get_by_id(pool, book_id).await?;

    match mode {
        BatchTagMode::Replace => {
            BookRepository::clear_tags(pool, book_id).await?;
            for &tag_id in tag_ids {
                BookRepository::add_tag(pool, book_id, tag_id).await?;
            }
        }
        BatchTagMode::Add => {
            for &tag_id in tag_ids {
                BookRepository::add_tag(pool, book_id, tag_id).await?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::extract::{Path, State};
    use axum::Json;
    use tempfile::TempDir;

    use archivis_auth::{AuthService, LocalAuthAdapter};
    use archivis_core::models::{
        Author, Book, FieldProvenance, Identifier, IdentifierType, MetadataSource, MetadataStatus,
        ResolutionOutcome, ResolutionState, Series, User, UserRole,
    };
    use archivis_db::{create_pool, run_migrations};
    use archivis_metadata::{MetadataResolver, ProviderRegistry};
    use archivis_storage::local::LocalStorage;
    use archivis_tasks::merge::MergeService;
    use archivis_tasks::queue::TaskQueue;
    use archivis_tasks::resolve::ResolutionService;

    use crate::books::types::{BookAuthorLink, BookSeriesLink, FieldProtectionRequest};
    use crate::settings::service::ConfigService;
    use crate::state::{ApiConfig, AppState};

    use super::*;

    struct TestSettings;

    impl archivis_core::settings::SettingsReader for TestSettings {
        fn get_setting(&self, _key: &str) -> Option<serde_json::Value> {
            None
        }
    }

    async fn test_state(tmp: &TempDir) -> AppState {
        let db_path = tmp.path().join("test.db");
        let storage_dir = tmp.path().join("books");
        let db_pool = create_pool(&db_path).await.unwrap();
        run_migrations(&db_pool).await.unwrap();

        let storage = LocalStorage::new(&storage_dir).await.unwrap();
        let auth_adapter = LocalAuthAdapter::new(db_pool.clone());
        let auth_service = AuthService::new(db_pool.clone(), auth_adapter);
        let (task_queue, mut rx) = TaskQueue::new(db_pool.clone());
        tokio::spawn(async move { while rx.recv().await.is_some() {} });

        let provider_registry = Arc::new(ProviderRegistry::new());
        let resolver = Arc::new(MetadataResolver::new(
            Arc::clone(&provider_registry),
            Arc::new(TestSettings),
        ));
        let resolve_service = Arc::new(ResolutionService::new(
            db_pool.clone(),
            resolver,
            storage.clone(),
            tmp.path().to_path_buf(),
        ));
        let merge_service = Arc::new(MergeService::new(
            db_pool.clone(),
            storage.clone(),
            tmp.path().to_path_buf(),
        ));
        let config_service = Arc::new(ConfigService::new(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            db_pool.clone(),
        ));

        AppState::new(
            db_pool,
            Arc::new(task_queue),
            auth_service,
            storage,
            provider_registry,
            resolve_service,
            merge_service,
            ApiConfig {
                data_dir: tmp.path().to_path_buf(),
                frontend_dir: None,
            },
            config_service,
            None,
            None,
        )
    }

    fn auth_user() -> AuthUser {
        AuthUser(User::new(
            "tester".into(),
            "password-hash".into(),
            UserRole::Admin,
        ))
    }

    async fn create_resolved_book(pool: &archivis_db::DbPool, title: &str) -> (Book, Identifier) {
        let mut book = Book::new(title);
        book.metadata_status = MetadataStatus::Identified;
        book.resolution_state = ResolutionState::Done;
        BookRepository::create(pool, &book).await.unwrap();

        let author = Author::new("Seed Author");
        AuthorRepository::create(pool, &author).await.unwrap();
        BookRepository::add_author(pool, book.id, author.id, "author", 0)
            .await
            .unwrap();

        let identifier = Identifier::new(
            book.id,
            IdentifierType::Asin,
            "B000SEED",
            MetadataSource::Provider("seed".into()),
            0.7,
        );
        IdentifierRepository::create(pool, &identifier)
            .await
            .unwrap();

        (book, identifier)
    }

    #[tokio::test]
    async fn update_book_title_sets_user_provenance_and_pending() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, _) = create_resolved_book(state.db_pool(), "Before").await;

        let _ = update_book(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(UpdateBookRequest {
                title: Some("After".into()),
                subtitle: None,
                description: None,
                language: None,
                publication_date: None,
                rating: None,
                page_count: None,
                publisher_id: None,
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        assert_eq!(updated.title, "After");
        assert_eq!(updated.resolution_state, ResolutionState::Pending);
        assert_eq!(
            updated.resolution_requested_reason.as_deref(),
            Some(USER_EDIT_TRIGGER)
        );
        assert_eq!(
            updated.metadata_provenance.title.as_ref().unwrap().origin,
            MetadataSource::User
        );
        assert!(
            updated
                .metadata_provenance
                .title
                .as_ref()
                .unwrap()
                .protected
        );
    }

    #[tokio::test]
    async fn set_book_authors_sets_user_provenance_and_pending() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, _) = create_resolved_book(state.db_pool(), "Author Edit").await;
        let replacement = Author::new("Replacement Author");
        AuthorRepository::create(state.db_pool(), &replacement)
            .await
            .unwrap();

        let _ = set_book_authors(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(SetBookAuthorsRequest {
                authors: vec![BookAuthorLink {
                    author_id: replacement.id,
                    role: "author".into(),
                    position: 0,
                }],
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        let with_relations = BookRepository::get_with_relations(state.db_pool(), book.id)
            .await
            .unwrap();
        assert_eq!(updated.resolution_state, ResolutionState::Pending);
        assert_eq!(
            updated.metadata_provenance.authors.as_ref().unwrap().origin,
            MetadataSource::User
        );
        assert!(
            updated
                .metadata_provenance
                .authors
                .as_ref()
                .unwrap()
                .protected
        );
        assert_eq!(with_relations.authors.len(), 1);
        assert_eq!(with_relations.authors[0].author.id, replacement.id);
    }

    #[tokio::test]
    async fn set_book_series_sets_user_provenance_and_pending() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, _) = create_resolved_book(state.db_pool(), "Series Edit").await;
        let series = Series::new("Replacement Series");
        SeriesRepository::create(state.db_pool(), &series)
            .await
            .unwrap();

        let _ = set_book_series(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(SetBookSeriesRequest {
                series: vec![BookSeriesLink {
                    series_id: series.id,
                    position: Some(2.0),
                }],
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        let with_relations = BookRepository::get_with_relations(state.db_pool(), book.id)
            .await
            .unwrap();
        assert_eq!(updated.resolution_state, ResolutionState::Pending);
        assert_eq!(
            updated.metadata_provenance.series.as_ref().unwrap().origin,
            MetadataSource::User
        );
        assert!(
            updated
                .metadata_provenance
                .series
                .as_ref()
                .unwrap()
                .protected
        );
        assert_eq!(with_relations.series.len(), 1);
        assert_eq!(with_relations.series[0].series.id, series.id);
    }

    #[tokio::test]
    async fn update_identifier_sets_user_source_and_pending() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let (book, identifier) = create_resolved_book(state.db_pool(), "Identifier Edit").await;

        let _ = update_identifier(
            State(state.clone()),
            auth_user(),
            Path((book.id, identifier.id)),
            Json(UpdateIdentifierRequest {
                identifier_type: Some(IdentifierType::Asin),
                value: Some("B000UPDATED".into()),
            }),
        )
        .await
        .unwrap();

        let updated_book = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        let updated_identifier = IdentifierRepository::get_by_id(state.db_pool(), identifier.id)
            .await
            .unwrap();

        assert_eq!(updated_book.resolution_state, ResolutionState::Pending);
        assert_eq!(
            updated_book.resolution_requested_reason.as_deref(),
            Some(USER_EDIT_TRIGGER)
        );
        assert_eq!(updated_identifier.value, "B000UPDATED");
        assert_eq!(updated_identifier.source, MetadataSource::User);
        assert!((updated_identifier.confidence - 1.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn get_book_returns_resolution_fields() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let mut book = Book::new("Resolution Detail");
        book.metadata_status = MetadataStatus::NeedsReview;
        book.resolution_state = ResolutionState::Done;
        book.resolution_outcome = Some(ResolutionOutcome::Disputed);
        book.metadata_locked = true;
        BookRepository::create(state.db_pool(), &book)
            .await
            .unwrap();

        let Json(detail) = get_book(State(state.clone()), auth_user(), Path(book.id))
            .await
            .unwrap();

        assert_eq!(detail.metadata_status, MetadataStatus::NeedsReview);
        assert_eq!(detail.resolution_state, ResolutionState::Done);
        assert_eq!(detail.resolution_outcome, Some(ResolutionOutcome::Disputed));
        assert!(detail.metadata_locked);
        assert!(detail.metadata_provenance.title.is_none());
    }

    #[tokio::test]
    async fn lock_and_unlock_metadata_toggle_book_lock() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;
        let book = Book::new("Toggle Lock");
        BookRepository::create(state.db_pool(), &book)
            .await
            .unwrap();

        let Json(locked) = lock_metadata(State(state.clone()), auth_user(), Path(book.id))
            .await
            .unwrap();
        assert!(locked.metadata_locked);

        let Json(unlocked) = unlock_metadata(State(state.clone()), auth_user(), Path(book.id))
            .await
            .unwrap();
        assert!(!unlocked.metadata_locked);
        assert_eq!(unlocked.resolution_state, ResolutionState::Pending);

        let refreshed = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        assert_eq!(
            refreshed.resolution_requested_reason.as_deref(),
            Some(UNLOCK_TRIGGER)
        );
    }

    #[tokio::test]
    async fn protect_fields_preserves_existing_origin() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let mut book = Book::new("Protect Me");
        book.metadata_provenance.title = Some(FieldProvenance {
            origin: MetadataSource::Provider("seed".into()),
            protected: false,
        });
        BookRepository::create(state.db_pool(), &book)
            .await
            .unwrap();

        let _ = protect_fields(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(FieldProtectionRequest {
                fields: vec!["title".into()],
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        let title = updated.metadata_provenance.title.as_ref().unwrap();
        assert_eq!(title.origin, MetadataSource::Provider("seed".into()));
        assert!(title.protected);
        assert_eq!(updated.resolution_state, ResolutionState::Pending);
        assert_eq!(
            updated.resolution_requested_reason.as_deref(),
            Some(PROTECT_FIELDS_TRIGGER)
        );
    }

    #[tokio::test]
    async fn unprotect_fields_clears_protection_without_dropping_origin() {
        let tmp = TempDir::new().unwrap();
        let state = test_state(&tmp).await;

        let mut book = Book::new("Unprotect Me");
        book.metadata_provenance.title = Some(FieldProvenance {
            origin: MetadataSource::Embedded,
            protected: true,
        });
        BookRepository::create(state.db_pool(), &book)
            .await
            .unwrap();

        let _ = unprotect_fields(
            State(state.clone()),
            auth_user(),
            Path(book.id),
            Json(FieldProtectionRequest {
                fields: vec!["title".into()],
            }),
        )
        .await
        .unwrap();

        let updated = BookRepository::get_by_id(state.db_pool(), book.id)
            .await
            .unwrap();
        let title = updated.metadata_provenance.title.as_ref().unwrap();
        assert_eq!(title.origin, MetadataSource::Embedded);
        assert!(!title.protected);
        assert_eq!(updated.resolution_state, ResolutionState::Pending);
        assert_eq!(
            updated.resolution_requested_reason.as_deref(),
            Some(UNPROTECT_FIELDS_TRIGGER)
        );
    }
}
