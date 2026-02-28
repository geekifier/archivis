use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use archivis_core::models::{
    Book, BookFile, BookFormat, Identifier, IdentifierType, MetadataSource, MetadataStatus, Tag,
};
use archivis_db::{BookAuthorEntry, BookSeriesEntry, BookWithRelations, PaginatedResult};

/// Deserializer that distinguishes between absent, `null`, and a present value.
/// - Field absent in JSON: `None` (no change)
/// - Field present as `null`: `Some(None)` (clear the value)
/// - Field present with value: `Some(Some(value))` (set the value)
#[allow(clippy::option_option)]
fn deserialize_optional_field<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(Some(Option::deserialize(deserializer)?))
}

// ── Query Parameters ────────────────────────────────────────────

/// Query parameters for `GET /api/books`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct BookListParams {
    /// Page number (1-indexed).
    pub page: Option<u32>,
    /// Results per page (max 100).
    pub per_page: Option<u32>,
    /// Sort field: `added_at`, title, `sort_title`, `updated_at`, rating, `metadata_status`.
    pub sort_by: Option<String>,
    /// Sort direction: asc or desc.
    pub sort_order: Option<String>,
    /// Full-text search query.
    pub q: Option<String>,
    /// Filter by book format (e.g. epub, pdf).
    pub format: Option<String>,
    /// Filter by metadata status.
    pub status: Option<String>,
    /// Filter by tag name.
    pub tag: Option<String>,
    /// Filter by author ID.
    pub author_id: Option<Uuid>,
    /// Filter by series ID.
    pub series_id: Option<Uuid>,
    /// Comma-separated list of relations to include: authors, series, tags, files.
    pub include: Option<String>,
}

/// Query parameters for `GET /api/books/{id}/cover`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct CoverParams {
    /// Thumbnail size: sm, md, lg, or original (default: original).
    pub size: Option<String>,
}

// ── Request Bodies ──────────────────────────────────────────────

/// Request body for `PUT /api/books/{id}`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateBookRequest {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    #[schema(value_type = Option<String>, example = "2024-01-15")]
    pub publication_date: Option<NaiveDate>,
    #[validate(range(min = 0.0, max = 5.0))]
    pub rating: Option<f32>,
    pub page_count: Option<i32>,
    #[schema(value_type = Option<String>)]
    pub metadata_status: Option<MetadataStatus>,
    /// Set or clear the publisher. Pass a UUID to set, `null` to clear.
    /// Omit entirely to leave unchanged.
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    #[schema(value_type = Option<String>)]
    #[allow(clippy::option_option)]
    pub publisher_id: Option<Option<Uuid>>,
}

/// A single author link in a set-authors request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BookAuthorLink {
    pub author_id: Uuid,
    #[serde(default = "default_author_role")]
    pub role: String,
    #[serde(default)]
    pub position: i32,
}

fn default_author_role() -> String {
    "author".into()
}

/// Request body for `POST /api/books/{id}/authors`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetBookAuthorsRequest {
    pub authors: Vec<BookAuthorLink>,
}

/// A single series link in a set-series request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BookSeriesLink {
    pub series_id: Uuid,
    pub position: Option<f64>,
}

/// Request body for `POST /api/books/{id}/series`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetBookSeriesRequest {
    pub series: Vec<BookSeriesLink>,
}

/// A single tag link in a set-tags request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BookTagLink {
    pub tag_id: Option<Uuid>,
    pub name: Option<String>,
    pub category: Option<String>,
}

/// Request body for `POST /api/books/{id}/tags`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetBookTagsRequest {
    pub tags: Vec<BookTagLink>,
}

/// Request body for `POST /api/books/{id}/identifiers`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct AddIdentifierRequest {
    /// The type of identifier (e.g. isbn13, isbn10, asin).
    #[schema(value_type = String)]
    pub identifier_type: IdentifierType,
    /// The identifier value.
    pub value: String,
}

/// Request body for `PUT /api/books/{id}/identifiers/{identifier_id}`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateIdentifierRequest {
    /// Updated identifier type.
    #[schema(value_type = Option<String>)]
    pub identifier_type: Option<IdentifierType>,
    /// Updated identifier value.
    pub value: Option<String>,
}

/// Request body for `POST /api/books/batch-update`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct BatchUpdateBooksRequest {
    /// IDs of books to update (max 100).
    pub book_ids: Vec<Uuid>,
    /// Fields to update on all selected books.
    #[validate(nested)]
    pub updates: BatchBookFields,
}

/// Fields that can be updated in bulk.
/// Each field follows the same None/Some/Some(None) pattern as `UpdateBookRequest`:
/// absent = no change, present with value = set, present with null = clear.
///
/// `title` and `description` are intentionally excluded -- setting the same
/// title on multiple books doesn't make sense.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct BatchBookFields {
    /// Set language for all books.
    pub language: Option<String>,
    /// Set metadata status for all books.
    #[schema(value_type = Option<String>)]
    pub metadata_status: Option<MetadataStatus>,
    /// Set rating for all books (0.0 -- 5.0).
    #[validate(range(min = 0.0, max = 5.0))]
    pub rating: Option<f32>,
    /// Set or clear the publisher for all books.
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    #[schema(value_type = Option<String>)]
    #[allow(clippy::option_option)]
    pub publisher_id: Option<Option<Uuid>>,
}

/// Request body for `POST /api/books/batch-tags`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchSetTagsRequest {
    /// IDs of books to update (max 100).
    pub book_ids: Vec<Uuid>,
    /// Tags to set or add.
    pub tags: Vec<BookTagLink>,
    /// "replace" (clear existing and set these) or "add" (add without removing existing).
    pub mode: BatchTagMode,
}

/// Mode for batch tag operations.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BatchTagMode {
    /// Clear all existing tags and set these.
    Replace,
    /// Add tags without removing existing ones.
    Add,
}

/// Response for batch update operations.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchUpdateResponse {
    /// Number of books successfully updated.
    pub updated_count: u32,
    /// Per-book errors (if any).
    pub errors: Vec<BatchUpdateError>,
}

/// Response for batch tag update operations.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchTagsResponse {
    /// Number of books successfully updated.
    pub updated_count: u32,
    /// Per-book errors (if any).
    pub errors: Vec<BatchUpdateError>,
}

/// An error that occurred while updating a single book in a batch.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchUpdateError {
    /// The book ID that failed.
    pub book_id: Uuid,
    /// Human-readable error message.
    pub error: String,
}

// ── Response Types ──────────────────────────────────────────────

/// Lightweight book summary for list responses.
#[derive(Debug, Serialize, ToSchema)]
pub struct BookSummary {
    pub id: Uuid,
    pub title: String,
    pub subtitle: Option<String>,
    pub sort_title: String,
    pub description: Option<String>,
    pub language: Option<String>,
    #[schema(value_type = Option<String>)]
    pub publication_date: Option<NaiveDate>,
    pub added_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub rating: Option<f32>,
    pub page_count: Option<i32>,
    #[schema(value_type = String)]
    pub metadata_status: MetadataStatus,
    pub metadata_confidence: f32,
    pub has_cover: bool,
    /// Populated when `?include=authors`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<AuthorEntry>>,
    /// Populated when `?include=series`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<SeriesEntry>>,
    /// Populated when `?include=tags`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<TagEntry>>,
    /// Populated when `?include=files`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<FileEntry>>,
}

/// Full book detail with all relations.
#[derive(Debug, Serialize, ToSchema)]
pub struct BookDetail {
    pub id: Uuid,
    pub title: String,
    pub subtitle: Option<String>,
    pub sort_title: String,
    pub description: Option<String>,
    pub language: Option<String>,
    #[schema(value_type = Option<String>)]
    pub publication_date: Option<NaiveDate>,
    pub publisher_id: Option<Uuid>,
    pub publisher_name: Option<String>,
    pub added_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub rating: Option<f32>,
    pub page_count: Option<i32>,
    #[schema(value_type = String)]
    pub metadata_status: MetadataStatus,
    pub metadata_confidence: f32,
    pub has_cover: bool,
    pub authors: Vec<AuthorEntry>,
    pub series: Vec<SeriesEntry>,
    pub tags: Vec<TagEntry>,
    pub files: Vec<FileEntry>,
    pub identifiers: Vec<IdentifierEntry>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorEntry {
    pub id: Uuid,
    pub name: String,
    pub sort_name: String,
    pub role: String,
    pub position: i32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SeriesEntry {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub position: Option<f64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TagEntry {
    pub id: Uuid,
    pub name: String,
    pub category: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FileEntry {
    pub id: Uuid,
    #[schema(value_type = String)]
    pub format: BookFormat,
    pub format_version: Option<String>,
    pub file_size: i64,
    pub hash: String,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IdentifierEntry {
    pub id: Uuid,
    #[schema(value_type = String)]
    pub identifier_type: IdentifierType,
    pub value: String,
    #[schema(value_type = Object)]
    pub source: MetadataSource,
    pub confidence: f32,
}

/// Paginated list of books.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaginatedBooks {
    pub items: Vec<BookSummary>,
    pub total: u32,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
}

// ── Conversions ─────────────────────────────────────────────────

impl From<BookWithRelations> for BookDetail {
    fn from(bwr: BookWithRelations) -> Self {
        Self {
            id: bwr.book.id,
            title: bwr.book.title,
            subtitle: bwr.book.subtitle,
            sort_title: bwr.book.sort_title,
            description: bwr.book.description,
            language: bwr.book.language,
            publication_date: bwr.book.publication_date,
            publisher_id: bwr.book.publisher_id,
            publisher_name: bwr.publisher_name,
            added_at: bwr.book.added_at,
            updated_at: bwr.book.updated_at,
            rating: bwr.book.rating,
            page_count: bwr.book.page_count,
            metadata_status: bwr.book.metadata_status,
            metadata_confidence: bwr.book.metadata_confidence,
            has_cover: bwr.book.cover_path.is_some(),
            authors: bwr.authors.into_iter().map(AuthorEntry::from).collect(),
            series: bwr.series.into_iter().map(SeriesEntry::from).collect(),
            tags: bwr.tags.into_iter().map(TagEntry::from).collect(),
            files: bwr.files.into_iter().map(FileEntry::from).collect(),
            identifiers: bwr
                .identifiers
                .into_iter()
                .map(IdentifierEntry::from)
                .collect(),
        }
    }
}

impl From<Book> for BookSummary {
    fn from(book: Book) -> Self {
        Self {
            id: book.id,
            title: book.title,
            subtitle: book.subtitle,
            sort_title: book.sort_title,
            description: book.description,
            language: book.language,
            publication_date: book.publication_date,
            added_at: book.added_at,
            updated_at: book.updated_at,
            rating: book.rating,
            page_count: book.page_count,
            metadata_status: book.metadata_status,
            metadata_confidence: book.metadata_confidence,
            has_cover: book.cover_path.is_some(),
            authors: None,
            series: None,
            tags: None,
            files: None,
        }
    }
}

impl From<BookAuthorEntry> for AuthorEntry {
    #[allow(clippy::cast_possible_truncation)]
    fn from(entry: BookAuthorEntry) -> Self {
        Self {
            id: entry.author.id,
            name: entry.author.name,
            sort_name: entry.author.sort_name,
            role: entry.role,
            position: entry.position as i32,
        }
    }
}

impl From<BookSeriesEntry> for SeriesEntry {
    fn from(entry: BookSeriesEntry) -> Self {
        Self {
            id: entry.series.id,
            name: entry.series.name,
            description: entry.series.description,
            position: entry.position,
        }
    }
}

impl From<Tag> for TagEntry {
    fn from(tag: Tag) -> Self {
        Self {
            id: tag.id,
            name: tag.name,
            category: tag.category,
        }
    }
}

impl From<BookFile> for FileEntry {
    fn from(file: BookFile) -> Self {
        Self {
            id: file.id,
            format: file.format,
            format_version: file.format_version,
            file_size: file.file_size,
            hash: file.hash,
            added_at: file.added_at,
        }
    }
}

impl From<Identifier> for IdentifierEntry {
    fn from(ident: Identifier) -> Self {
        Self {
            id: ident.id,
            identifier_type: ident.identifier_type,
            value: ident.value,
            source: ident.source,
            confidence: ident.confidence,
        }
    }
}

impl<T: Into<BookSummary>> From<PaginatedResult<T>> for PaginatedBooks {
    fn from(result: PaginatedResult<T>) -> Self {
        Self {
            items: result.items.into_iter().map(Into::into).collect(),
            total: result.total,
            page: result.page,
            per_page: result.per_page,
            total_pages: result.total_pages,
        }
    }
}
