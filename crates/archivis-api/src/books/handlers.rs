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
use archivis_core::models::{Identifier, IdentifierType, MetadataSource};

use archivis_db::{
    AuthorRepository, BookFileRepository, BookFilter, BookRepository, IdentifierRepository,
    PaginationParams, SeriesRepository, SortOrder, TagRepository,
};
use archivis_storage::StorageBackend;

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    AddIdentifierRequest, BookDetail, BookListParams, BookSummary, CoverParams, PaginatedBooks,
    SetBookAuthorsRequest, SetBookSeriesRequest, SetBookTagsRequest, UpdateBookRequest,
    UpdateIdentifierRequest,
};

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

    // Always strip dangerous content from user-submitted text
    let sanitize_opts = SanitizeOptions::default();

    if let Some(ref title) = body.title {
        let clean = sanitize_text(title, &sanitize_opts).unwrap_or_default();
        if clean.is_empty() {
            return Err(ApiError::Validation("title must not be empty".into()));
        }
        book.set_title(clean);
    }
    if let Some(ref description) = body.description {
        book.description = sanitize_text(description, &sanitize_opts);
    }
    if let Some(language) = body.language {
        book.language = Some(language).filter(|s| !s.is_empty());
    }
    if let Some(pub_date) = body.publication_date {
        book.publication_date = Some(pub_date);
    }
    if let Some(rating) = body.rating {
        book.rating = Some(rating);
    }
    if let Some(page_count) = body.page_count {
        book.page_count = Some(page_count);
    }
    if let Some(status) = body.metadata_status {
        book.metadata_status = status;
    }
    // publisher_id: Some(Some(id)) = set, Some(None) = clear, None = no change
    if let Some(pub_id) = body.publisher_id {
        book.publisher_id = pub_id;
    }

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
            .as_secs(),
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

    // Verify book exists
    BookRepository::get_by_id(pool, id).await?;

    // Verify each author exists
    for link in &body.authors {
        AuthorRepository::get_by_id(pool, link.author_id).await?;
    }

    // Replace all author links
    BookRepository::clear_authors(pool, id).await?;
    for link in &body.authors {
        BookRepository::add_author(pool, id, link.author_id, &link.role, link.position).await?;
    }

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

    // Verify book exists
    BookRepository::get_by_id(pool, id).await?;

    // Verify each series exists
    for link in &body.series {
        SeriesRepository::get_by_id(pool, link.series_id).await?;
    }

    // Replace all series links
    BookRepository::clear_series(pool, id).await?;
    for link in &body.series {
        BookRepository::add_series(pool, id, link.series_id, link.position).await?;
    }

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

    Ok(StatusCode::NO_CONTENT)
}
