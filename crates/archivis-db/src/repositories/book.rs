use archivis_core::errors::DbError;
use archivis_core::models::{Author, Book, BookFile, Identifier, MetadataStatus, Series, Tag};
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::types::{BookFilter, PaginatedResult, PaginationParams, SortOrder};

/// A book with all its related entities loaded.
#[derive(Debug, Clone)]
pub struct BookWithRelations {
    pub book: Book,
    pub authors: Vec<BookAuthorEntry>,
    pub series: Vec<BookSeriesEntry>,
    pub files: Vec<BookFile>,
    pub identifiers: Vec<Identifier>,
    pub tags: Vec<Tag>,
    pub publisher_name: Option<String>,
}

/// An author entry with role and position in a book.
#[derive(Debug, Clone)]
pub struct BookAuthorEntry {
    pub author: Author,
    pub role: String,
    pub position: i64,
}

/// A series entry with position.
#[derive(Debug, Clone)]
pub struct BookSeriesEntry {
    pub series: Series,
    pub position: Option<f64>,
}

/// A book with its author names pre-loaded (for duplicate detection).
#[derive(Debug, Clone)]
pub struct BookWithAuthors {
    pub book: Book,
    pub author_names: Vec<String>,
}

pub struct BookRepository;

// ── Book list helper macros ────────────────────────────────────
//
// Each sort variant needs its own `query_as!()` invocation because
// the macro requires a string literal for the SQL. The helper macros
// reduce boilerplate: one for queries without FTS, one with FTS JOIN.

/// Fetch book rows WITHOUT full-text search.
macro_rules! fetch_books {
    ($sql:literal, $pool:expr, $fmt:expr, $status:expr, $author:expr, $series:expr, $tags:expr, $publisher:expr, $limit:expr, $offset:expr) => {
        sqlx::query_as!(
            BookRow, $sql, $fmt, $fmt, $status, $status, $author, $author, $series, $series, $tags,
            $tags, $publisher, $publisher, $limit, $offset,
        )
        .fetch_all($pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
    };
}

/// Fetch book rows WITH full-text search JOIN.
macro_rules! fetch_books_fts {
    ($sql:literal, $pool:expr, $fts:expr, $fmt:expr, $status:expr, $author:expr, $series:expr, $tags:expr, $publisher:expr, $limit:expr, $offset:expr) => {
        sqlx::query_as!(
            BookRow, $sql, $fts, $fmt, $fmt, $status, $status, $author, $author, $series, $series,
            $tags, $tags, $publisher, $publisher, $limit, $offset,
        )
        .fetch_all($pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
    };
}

// ── SQL fragments (as constants for documentation; actual SQL is in macro call sites) ──
//
// Non-FTS WHERE clause (each filter is opt-in via IS NULL OR):
//   WHERE (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?))
//   AND (? IS NULL OR b.metadata_status = ?)
//   AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?))
//   AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?))
//   AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?))))
//   AND (? IS NULL OR b.publisher_id = ?)
//
// FTS variant prepends:
//   JOIN books_fts ON books_fts.book_id = b.id WHERE books_fts MATCH ?
//   AND (... same filters ...)

impl BookRepository {
    pub async fn create(pool: &SqlitePool, book: &Book) -> Result<(), DbError> {
        let id = book.id.to_string();
        let publisher_id = book.publisher_id.map(|p| p.to_string());
        let pub_date = book.publication_date.map(|d| d.to_string());
        let added_at = book.added_at.to_rfc3339();
        let updated_at = book.updated_at.to_rfc3339();
        let status = serde_json::to_value(book.metadata_status)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "unidentified".into());

        sqlx::query!(
            "INSERT INTO books (id, title, sort_title, description, language, publication_date, publisher_id, added_at, updated_at, rating, page_count, metadata_status, metadata_confidence, cover_path)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            id,
            book.title,
            book.sort_title,
            book.description,
            book.language,
            pub_date,
            publisher_id,
            added_at,
            updated_at,
            book.rating,
            book.page_count,
            status,
            book.metadata_confidence,
            book.cover_path,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Book, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            BookRow,
            "SELECT id, title, sort_title, description, language, publication_date, publisher_id, added_at, updated_at, rating, page_count, metadata_status, metadata_confidence, cover_path FROM books WHERE id = ?",
            id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "book",
            id: id_str,
        })?;

        row.into_book()
    }

    #[allow(clippy::too_many_lines)] // 20 sort-variant match arms required by compile-time checked macros
    pub async fn list(
        pool: &SqlitePool,
        params: &PaginationParams,
        filter: &BookFilter,
    ) -> Result<PaginatedResult<Book>, DbError> {
        // Prepare filter binds — None disables the filter via `IS NULL OR` short-circuit.
        let fmt_filter = filter.format.as_ref().map(|f| {
            serde_json::to_value(f)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default()
        });
        let status_filter = filter.status.as_ref().map(|s| {
            serde_json::to_value(s)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default()
        });
        let author_filter = filter.author_id.clone();
        let series_filter = filter.series_id.clone();
        let publisher_filter = filter.publisher_id.clone();

        // Tags: convert Vec<String> to a JSON array string for json_each()
        let tags_json = filter
            .tags
            .as_ref()
            .filter(|t| !t.is_empty())
            .map(|t| serde_json::to_string(t).unwrap_or_default());

        let limit = params.per_page;
        let offset = params.offset();

        // FTS query: wrap in double quotes for phrase matching, escape internal quotes
        let fts_query = filter
            .query
            .as_ref()
            .filter(|q| !q.is_empty())
            .map(|q| format!("\"{}\"", q.replace('"', "\"\"")));

        let (total, rows) = if let Some(ref fts_q) = fts_query {
            // ── FTS path: JOIN books_fts ────────────────────────────
            let total = sqlx::query_scalar!(
                "SELECT COUNT(*) FROM books b JOIN books_fts ON books_fts.book_id = b.id WHERE books_fts MATCH ? AND (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?)",
                fts_q,
                fmt_filter, fmt_filter,
                status_filter, status_filter,
                author_filter, author_filter,
                series_filter, series_filter,
                tags_json, tags_json,
                publisher_filter, publisher_filter,
            )
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

            let rows = match (params.sort_by.as_str(), params.sort_order) {
                ("title" | "sort_title", SortOrder::Asc) => fetch_books_fts!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b JOIN books_fts ON books_fts.book_id = b.id WHERE books_fts MATCH ? AND (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.sort_title ASC LIMIT ? OFFSET ?",
                    pool, fts_q, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("title" | "sort_title", SortOrder::Desc) => fetch_books_fts!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b JOIN books_fts ON books_fts.book_id = b.id WHERE books_fts MATCH ? AND (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.sort_title DESC LIMIT ? OFFSET ?",
                    pool, fts_q, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("updated_at", SortOrder::Asc) => fetch_books_fts!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b JOIN books_fts ON books_fts.book_id = b.id WHERE books_fts MATCH ? AND (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.updated_at ASC LIMIT ? OFFSET ?",
                    pool, fts_q, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("updated_at", SortOrder::Desc) => fetch_books_fts!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b JOIN books_fts ON books_fts.book_id = b.id WHERE books_fts MATCH ? AND (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.updated_at DESC LIMIT ? OFFSET ?",
                    pool, fts_q, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("rating", SortOrder::Asc) => fetch_books_fts!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b JOIN books_fts ON books_fts.book_id = b.id WHERE books_fts MATCH ? AND (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.rating ASC LIMIT ? OFFSET ?",
                    pool, fts_q, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("rating", SortOrder::Desc) => fetch_books_fts!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b JOIN books_fts ON books_fts.book_id = b.id WHERE books_fts MATCH ? AND (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.rating DESC LIMIT ? OFFSET ?",
                    pool, fts_q, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("metadata_status", SortOrder::Asc) => fetch_books_fts!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b JOIN books_fts ON books_fts.book_id = b.id WHERE books_fts MATCH ? AND (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.metadata_status ASC LIMIT ? OFFSET ?",
                    pool, fts_q, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("metadata_status", SortOrder::Desc) => fetch_books_fts!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b JOIN books_fts ON books_fts.book_id = b.id WHERE books_fts MATCH ? AND (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.metadata_status DESC LIMIT ? OFFSET ?",
                    pool, fts_q, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                (_, SortOrder::Desc) => fetch_books_fts!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b JOIN books_fts ON books_fts.book_id = b.id WHERE books_fts MATCH ? AND (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.added_at DESC LIMIT ? OFFSET ?",
                    pool, fts_q, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                // Default: added_at ASC
                _ => fetch_books_fts!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b JOIN books_fts ON books_fts.book_id = b.id WHERE books_fts MATCH ? AND (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.added_at ASC LIMIT ? OFFSET ?",
                    pool, fts_q, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
            };

            (total, rows)
        } else {
            // ── Non-FTS path ───────────────────────────────────────
            let total = sqlx::query_scalar!(
                "SELECT COUNT(*) FROM books b WHERE (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?)",
                fmt_filter, fmt_filter,
                status_filter, status_filter,
                author_filter, author_filter,
                series_filter, series_filter,
                tags_json, tags_json,
                publisher_filter, publisher_filter,
            )
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

            let rows = match (params.sort_by.as_str(), params.sort_order) {
                ("title" | "sort_title", SortOrder::Asc) => fetch_books!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b WHERE (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.sort_title ASC LIMIT ? OFFSET ?",
                    pool, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("title" | "sort_title", SortOrder::Desc) => fetch_books!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b WHERE (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.sort_title DESC LIMIT ? OFFSET ?",
                    pool, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("updated_at", SortOrder::Asc) => fetch_books!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b WHERE (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.updated_at ASC LIMIT ? OFFSET ?",
                    pool, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("updated_at", SortOrder::Desc) => fetch_books!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b WHERE (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.updated_at DESC LIMIT ? OFFSET ?",
                    pool, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("rating", SortOrder::Asc) => fetch_books!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b WHERE (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.rating ASC LIMIT ? OFFSET ?",
                    pool, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("rating", SortOrder::Desc) => fetch_books!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b WHERE (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.rating DESC LIMIT ? OFFSET ?",
                    pool, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("metadata_status", SortOrder::Asc) => fetch_books!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b WHERE (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.metadata_status ASC LIMIT ? OFFSET ?",
                    pool, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                ("metadata_status", SortOrder::Desc) => fetch_books!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b WHERE (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.metadata_status DESC LIMIT ? OFFSET ?",
                    pool, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                (_, SortOrder::Desc) => fetch_books!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b WHERE (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.added_at DESC LIMIT ? OFFSET ?",
                    pool, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
                // Default: added_at ASC
                _ => fetch_books!(
                    "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b WHERE (? IS NULL OR b.id IN (SELECT book_id FROM book_files WHERE format = ?)) AND (? IS NULL OR b.metadata_status = ?) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_authors WHERE author_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_series WHERE series_id = ?)) AND (? IS NULL OR b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN (SELECT value FROM json_each(?)))) AND (? IS NULL OR b.publisher_id = ?) ORDER BY b.added_at ASC LIMIT ? OFFSET ?",
                    pool, fmt_filter, status_filter, author_filter, series_filter, tags_json, publisher_filter, limit, offset
                ),
            };

            (total, rows)
        };

        let books = rows
            .into_iter()
            .map(BookRow::into_book)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(books, total as u32, params))
    }

    pub async fn update(pool: &SqlitePool, book: &Book) -> Result<(), DbError> {
        let id = book.id.to_string();
        let publisher_id = book.publisher_id.map(|p| p.to_string());
        let pub_date = book.publication_date.map(|d| d.to_string());
        let updated_at = Utc::now().to_rfc3339();
        let status = serde_json::to_value(book.metadata_status)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "unidentified".into());

        let result = sqlx::query!(
            "UPDATE books SET title = ?, sort_title = ?, description = ?, language = ?, publication_date = ?, publisher_id = ?, updated_at = ?, rating = ?, page_count = ?, metadata_status = ?, metadata_confidence = ?, cover_path = ? WHERE id = ?",
            book.title,
            book.sort_title,
            book.description,
            book.language,
            pub_date,
            publisher_id,
            updated_at,
            book.rating,
            book.page_count,
            status,
            book.metadata_confidence,
            book.cover_path,
            id,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "book", id });
        }

        Ok(())
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query!("DELETE FROM books WHERE id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "book",
                id: id_str,
            });
        }

        Ok(())
    }

    pub async fn search(
        pool: &SqlitePool,
        query: &str,
        params: &PaginationParams,
    ) -> Result<PaginatedResult<Book>, DbError> {
        let filter = BookFilter {
            query: Some(query.into()),
            ..BookFilter::default()
        };
        Self::list(pool, params, &filter).await
    }

    #[allow(clippy::too_many_lines)] // multiple sub-queries for related entities
    pub async fn get_with_relations(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<BookWithRelations, DbError> {
        let book = Self::get_by_id(pool, id).await?;
        let id_str = id.to_string();

        // Fetch authors
        let author_rows = sqlx::query_as!(
            BookAuthorRow,
            "SELECT a.id, a.name, a.sort_name, ba.role, ba.position FROM authors a JOIN book_authors ba ON ba.author_id = a.id WHERE ba.book_id = ? ORDER BY ba.position",
            id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let authors = author_rows
            .into_iter()
            .map(|r| {
                Ok(BookAuthorEntry {
                    author: Author {
                        id: Uuid::parse_str(&r.id)
                            .map_err(|e| DbError::Query(format!("invalid author UUID: {e}")))?,
                        name: r.name,
                        sort_name: r.sort_name,
                    },
                    role: r.role,
                    position: r.position,
                })
            })
            .collect::<Result<Vec<_>, DbError>>()?;

        // Fetch series
        let series_rows = sqlx::query_as!(
            BookSeriesRow,
            "SELECT s.id, s.name, s.description, bs.position FROM series s JOIN book_series bs ON bs.series_id = s.id WHERE bs.book_id = ? ORDER BY bs.position",
            id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let series = series_rows
            .into_iter()
            .map(|r| {
                Ok(BookSeriesEntry {
                    series: Series {
                        id: Uuid::parse_str(&r.id)
                            .map_err(|e| DbError::Query(format!("invalid series UUID: {e}")))?,
                        name: r.name,
                        description: r.description,
                    },
                    position: r.position,
                })
            })
            .collect::<Result<Vec<_>, DbError>>()?;

        // Fetch files
        let file_rows = sqlx::query_as!(
            BookFileRow,
            "SELECT id, book_id, format, storage_path, file_size, hash, added_at FROM book_files WHERE book_id = ?",
            id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let files = file_rows
            .into_iter()
            .map(BookFileRow::into_book_file)
            .collect::<Result<Vec<_>, _>>()?;

        // Fetch identifiers
        let ident_rows = sqlx::query_as!(
            IdentifierRow,
            "SELECT id, book_id, identifier_type, value, source_type, source_name, confidence FROM identifiers WHERE book_id = ?",
            id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let identifiers = ident_rows
            .into_iter()
            .map(IdentifierRow::into_identifier)
            .collect::<Result<Vec<_>, _>>()?;

        // Fetch tags
        let tag_rows = sqlx::query_as!(
            TagRow,
            "SELECT t.id, t.name, t.category FROM tags t JOIN book_tags bt ON bt.tag_id = t.id WHERE bt.book_id = ?",
            id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let tags = tag_rows
            .into_iter()
            .map(TagRow::into_tag)
            .collect::<Result<Vec<_>, _>>()?;

        // Fetch publisher name
        let publisher_name: Option<String> = if let Some(pid) = book.publisher_id {
            let pid_str = pid.to_string();
            sqlx::query_scalar!("SELECT name FROM publishers WHERE id = ?", pid_str)
                .fetch_optional(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?
        } else {
            None
        };

        Ok(BookWithRelations {
            book,
            authors,
            series,
            files,
            identifiers,
            tags,
            publisher_name,
        })
    }

    /// Remove all author links for a book.
    pub async fn clear_authors(pool: &SqlitePool, book_id: Uuid) -> Result<(), DbError> {
        let id_str = book_id.to_string();
        sqlx::query!("DELETE FROM book_authors WHERE book_id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(())
    }

    /// Remove all series links for a book.
    pub async fn clear_series(pool: &SqlitePool, book_id: Uuid) -> Result<(), DbError> {
        let id_str = book_id.to_string();
        sqlx::query!("DELETE FROM book_series WHERE book_id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(())
    }

    /// Remove all tag links for a book.
    pub async fn clear_tags(pool: &SqlitePool, book_id: Uuid) -> Result<(), DbError> {
        let id_str = book_id.to_string();
        sqlx::query!("DELETE FROM book_tags WHERE book_id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(())
    }

    /// Link a book to an author.
    pub async fn add_author(
        pool: &SqlitePool,
        book_id: Uuid,
        author_id: Uuid,
        role: &str,
        position: i32,
    ) -> Result<(), DbError> {
        let book_id_str = book_id.to_string();
        let author_id_str = author_id.to_string();
        sqlx::query!(
            "INSERT OR IGNORE INTO book_authors (book_id, author_id, role, position) VALUES (?, ?, ?, ?)",
            book_id_str,
            author_id_str,
            role,
            position,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// Link a book to a series.
    pub async fn add_series(
        pool: &SqlitePool,
        book_id: Uuid,
        series_id: Uuid,
        position: Option<f64>,
    ) -> Result<(), DbError> {
        let book_id_str = book_id.to_string();
        let series_id_str = series_id.to_string();
        sqlx::query!(
            "INSERT OR IGNORE INTO book_series (book_id, series_id, position) VALUES (?, ?, ?)",
            book_id_str,
            series_id_str,
            position,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// Update the position of a book within a series.
    pub async fn update_series_position(
        pool: &SqlitePool,
        book_id: Uuid,
        series_id: Uuid,
        position: Option<f64>,
    ) -> Result<(), DbError> {
        let book_id_str = book_id.to_string();
        let series_id_str = series_id.to_string();
        sqlx::query!(
            "UPDATE book_series SET position = ? WHERE book_id = ? AND series_id = ?",
            position,
            book_id_str,
            series_id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// Link a book to a tag.
    pub async fn add_tag(pool: &SqlitePool, book_id: Uuid, tag_id: Uuid) -> Result<(), DbError> {
        let book_id_str = book_id.to_string();
        let tag_id_str = tag_id.to_string();
        sqlx::query!(
            "INSERT OR IGNORE INTO book_tags (book_id, tag_id) VALUES (?, ?)",
            book_id_str,
            tag_id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// Find potential duplicate books by title similarity.
    ///
    /// Returns books whose normalized `sort_title` starts with the same
    /// prefix (first 3 characters), enabling efficient DB-level
    /// pre-filtering before expensive fuzzy matching in Rust.
    pub async fn find_potential_duplicates(
        pool: &SqlitePool,
        title: &str,
        limit: i64,
    ) -> Result<Vec<BookWithAuthors>, DbError> {
        let prefix = title
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(3)
            .collect::<String>();

        if prefix.len() < 3 {
            return Ok(Vec::new());
        }

        let like_pattern = format!("{prefix}%");

        let rows = sqlx::query_as!(
            DuplicateCandidateRow,
            r#"SELECT b.id, b.title, b.sort_title, b.description, b.language,
                      b.publication_date, b.publisher_id, b.added_at, b.updated_at,
                      b.rating, b.page_count, b.metadata_status, b.metadata_confidence,
                      b.cover_path,
                      GROUP_CONCAT(a.name, '||') as "author_names: String"
               FROM books b
               LEFT JOIN book_authors ba ON ba.book_id = b.id
               LEFT JOIN authors a ON a.id = ba.author_id
               WHERE LOWER(SUBSTR(b.sort_title, 1, 3)) LIKE ?
               GROUP BY b.id
               LIMIT ?"#,
            like_pattern,
            limit,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(|r| {
                let book = BookRow {
                    id: r.id,
                    title: r.title,
                    sort_title: r.sort_title,
                    description: r.description,
                    language: r.language,
                    publication_date: r.publication_date,
                    publisher_id: r.publisher_id,
                    added_at: r.added_at,
                    updated_at: r.updated_at,
                    rating: r.rating,
                    page_count: r.page_count,
                    metadata_status: r.metadata_status,
                    metadata_confidence: r.metadata_confidence,
                    cover_path: r.cover_path,
                }
                .into_book()?;

                let author_names = r
                    .author_names
                    .map(|names| {
                        names
                            .split("||")
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();

                Ok(BookWithAuthors { book, author_names })
            })
            .collect()
    }

    /// List books that need identification (below confidence threshold).
    pub async fn list_needing_identification(
        pool: &SqlitePool,
        confidence_threshold: f32,
        limit: i64,
    ) -> Result<Vec<Book>, DbError> {
        let status_identified = "identified";
        let rows = sqlx::query_as!(
            BookRow,
            "SELECT id, title, sort_title, description, language, publication_date, publisher_id, added_at, updated_at, rating, page_count, metadata_status, metadata_confidence, cover_path FROM books WHERE metadata_confidence < ? AND metadata_status != ? ORDER BY added_at DESC LIMIT ?",
            confidence_threshold,
            status_identified,
            limit,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(BookRow::into_book).collect()
    }
}

// ── Row types for sqlx mapping ──────────────────────────────────

/// Row type for the `find_potential_duplicates` query (book + grouped author names).
#[derive(sqlx::FromRow)]
struct DuplicateCandidateRow {
    id: String,
    title: String,
    sort_title: String,
    description: Option<String>,
    language: Option<String>,
    publication_date: Option<String>,
    publisher_id: Option<String>,
    added_at: String,
    updated_at: String,
    rating: Option<f64>,
    page_count: Option<i64>,
    metadata_status: String,
    metadata_confidence: f64,
    cover_path: Option<String>,
    author_names: Option<String>,
}

#[derive(sqlx::FromRow)]
struct BookRow {
    id: String,
    title: String,
    sort_title: String,
    description: Option<String>,
    language: Option<String>,
    publication_date: Option<String>,
    publisher_id: Option<String>,
    added_at: String,
    updated_at: String,
    rating: Option<f64>,
    page_count: Option<i64>,
    metadata_status: String,
    metadata_confidence: f64,
    cover_path: Option<String>,
}

impl BookRow {
    fn into_book(self) -> Result<Book, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
        let publisher_id = self
            .publisher_id
            .map(|p| Uuid::parse_str(&p))
            .transpose()
            .map_err(|e| DbError::Query(format!("invalid publisher UUID: {e}")))?;
        let publication_date = self
            .publication_date
            .map(|d| NaiveDate::parse_from_str(&d, "%Y-%m-%d"))
            .transpose()
            .map_err(|e| DbError::Query(format!("invalid publication_date: {e}")))?;
        let added_at = DateTime::parse_from_rfc3339(&self.added_at)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|e| DbError::Query(format!("invalid added_at: {e}")))?;
        let updated_at = DateTime::parse_from_rfc3339(&self.updated_at)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|e| DbError::Query(format!("invalid updated_at: {e}")))?;
        let metadata_status: MetadataStatus = self
            .metadata_status
            .parse()
            .map_err(|e: String| DbError::Query(e))?;

        #[allow(clippy::cast_possible_truncation)]
        Ok(Book {
            id,
            title: self.title,
            sort_title: self.sort_title,
            description: self.description,
            language: self.language,
            publication_date,
            publisher_id,
            added_at,
            updated_at,
            rating: self.rating.map(|r| r as f32),
            page_count: self.page_count.map(|p| p as i32),
            metadata_status,
            metadata_confidence: self.metadata_confidence as f32,
            cover_path: self.cover_path,
        })
    }
}

#[derive(sqlx::FromRow)]
struct BookAuthorRow {
    id: String,
    name: String,
    sort_name: String,
    role: String,
    position: i64,
}

#[derive(sqlx::FromRow)]
struct BookSeriesRow {
    id: String,
    name: String,
    description: Option<String>,
    position: Option<f64>,
}

#[derive(sqlx::FromRow)]
pub struct BookFileRow {
    pub id: String,
    pub book_id: String,
    pub format: String,
    pub storage_path: String,
    pub file_size: i64,
    pub hash: String,
    pub added_at: String,
}

impl BookFileRow {
    pub fn into_book_file(self) -> Result<BookFile, DbError> {
        use archivis_core::models::BookFormat;

        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid book_file UUID: {e}")))?;
        let book_id = Uuid::parse_str(&self.book_id)
            .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
        let format: BookFormat = self.format.parse().map_err(|e: String| DbError::Query(e))?;
        let added_at = DateTime::parse_from_rfc3339(&self.added_at)
            .map(|d| d.with_timezone(&Utc))
            .or_else(|_| {
                // Handle SQLite default timestamp format (with microseconds)
                chrono::NaiveDateTime::parse_from_str(&self.added_at, "%Y-%m-%dT%H:%M:%S%.fZ")
                    .map(|ndt| ndt.and_utc())
            })
            .map_err(|e| DbError::Query(format!("invalid added_at: {e}")))?;

        Ok(BookFile {
            id,
            book_id,
            format,
            storage_path: self.storage_path,
            file_size: self.file_size,
            hash: self.hash,
            added_at,
        })
    }
}

#[derive(sqlx::FromRow)]
pub struct IdentifierRow {
    pub id: String,
    pub book_id: String,
    pub identifier_type: String,
    pub value: String,
    pub source_type: String,
    pub source_name: Option<String>,
    pub confidence: f64,
}

impl IdentifierRow {
    pub fn into_identifier(self) -> Result<Identifier, DbError> {
        use archivis_core::models::{IdentifierType, MetadataSource};

        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid identifier UUID: {e}")))?;
        let book_id = Uuid::parse_str(&self.book_id)
            .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
        let identifier_type: IdentifierType = self
            .identifier_type
            .parse()
            .map_err(|e: String| DbError::Query(e))?;

        let source = match self.source_type.as_str() {
            "embedded" => MetadataSource::Embedded,
            "filename" => MetadataSource::Filename,
            "user" => MetadataSource::User,
            "provider" => MetadataSource::Provider(self.source_name.unwrap_or_default()),
            "content_scan" => MetadataSource::ContentScan,
            other => {
                return Err(DbError::Query(format!("unknown source_type: {other}")));
            }
        };

        #[allow(clippy::cast_possible_truncation)]
        Ok(Identifier {
            id,
            book_id,
            identifier_type,
            value: self.value,
            source,
            confidence: self.confidence as f32,
        })
    }
}

#[derive(sqlx::FromRow)]
pub struct TagRow {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
}

impl TagRow {
    pub fn into_tag(self) -> Result<Tag, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid tag UUID: {e}")))?;
        Ok(Tag {
            id,
            name: self.name,
            category: self.category,
        })
    }
}
