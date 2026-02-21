use archivis_core::errors::DbError;
use archivis_core::models::{Author, Book, BookFile, Identifier, MetadataStatus, Series, Tag};
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::types::{BookFilter, PaginatedResult, PaginationParams};

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
    pub position: i32,
}

/// A series entry with position.
#[derive(Debug, Clone)]
pub struct BookSeriesEntry {
    pub series: Series,
    pub position: Option<f64>,
}

pub struct BookRepository;

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

        sqlx::query(
            "INSERT INTO books (id, title, sort_title, description, language, publication_date, publisher_id, added_at, updated_at, rating, page_count, metadata_status, metadata_confidence, cover_path)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&book.title)
        .bind(&book.sort_title)
        .bind(&book.description)
        .bind(&book.language)
        .bind(&pub_date)
        .bind(&publisher_id)
        .bind(&added_at)
        .bind(&updated_at)
        .bind(book.rating)
        .bind(book.page_count)
        .bind(&status)
        .bind(book.metadata_confidence)
        .bind(&book.cover_path)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Book, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, BookRow>(
            "SELECT id, title, sort_title, description, language, publication_date, publisher_id, added_at, updated_at, rating, page_count, metadata_status, metadata_confidence, cover_path FROM books WHERE id = ?",
        )
        .bind(&id_str)
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "book",
            id: id_str,
        })?;

        row.into_book()
    }

    pub async fn list(
        pool: &SqlitePool,
        params: &PaginationParams,
        filter: &BookFilter,
    ) -> Result<PaginatedResult<Book>, DbError> {
        let mut where_clauses = Vec::new();
        let mut fts_join = String::new();

        if let Some(ref query) = filter.query {
            if !query.is_empty() {
                fts_join = " JOIN books_fts ON books_fts.book_id = b.id".into();
                // Escape FTS special chars to prevent injection
                let escaped = query.replace('"', "\"\"");
                where_clauses.push(format!("books_fts MATCH '\"{escaped}\"'"));
            }
        }

        if let Some(ref format) = filter.format {
            let fmt_str = serde_json::to_value(format)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();
            where_clauses.push(format!(
                "b.id IN (SELECT book_id FROM book_files WHERE format = '{fmt_str}')"
            ));
        }

        if let Some(ref status) = filter.status {
            let status_str = serde_json::to_value(status)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();
            where_clauses.push(format!("b.metadata_status = '{status_str}'"));
        }

        if let Some(ref author_id) = filter.author_id {
            where_clauses.push(format!(
                "b.id IN (SELECT book_id FROM book_authors WHERE author_id = '{author_id}')"
            ));
        }

        if let Some(ref series_id) = filter.series_id {
            where_clauses.push(format!(
                "b.id IN (SELECT book_id FROM book_series WHERE series_id = '{series_id}')"
            ));
        }

        if let Some(ref tags) = filter.tags {
            if !tags.is_empty() {
                let placeholders: Vec<String> = tags.iter().map(|t| format!("'{t}'")).collect();
                where_clauses.push(format!(
                    "b.id IN (SELECT book_id FROM book_tags WHERE tag_id IN ({}))",
                    placeholders.join(", ")
                ));
            }
        }

        let where_clause = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };

        // Validate sort_by against allowed columns
        let sort_col = match params.sort_by.as_str() {
            "title" | "sort_title" => "b.sort_title",
            "updated_at" => "b.updated_at",
            "rating" => "b.rating",
            "metadata_status" => "b.metadata_status",
            _ => "b.added_at",
        };
        let sort_dir = params.sort_order.as_sql();

        // Count query
        let count_sql = format!("SELECT COUNT(*) FROM books b{fts_join}{where_clause}");
        let total: i64 = sqlx::query_scalar(&count_sql)
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        // Data query
        let limit = params.per_page;
        let offset = params.offset();
        let data_sql = format!(
            "SELECT b.id, b.title, b.sort_title, b.description, b.language, b.publication_date, b.publisher_id, b.added_at, b.updated_at, b.rating, b.page_count, b.metadata_status, b.metadata_confidence, b.cover_path FROM books b{fts_join}{where_clause} ORDER BY {sort_col} {sort_dir} LIMIT {limit} OFFSET {offset}",
        );

        let rows = sqlx::query_as::<_, BookRow>(&data_sql)
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

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

        let result = sqlx::query(
            "UPDATE books SET title = ?, sort_title = ?, description = ?, language = ?, publication_date = ?, publisher_id = ?, updated_at = ?, rating = ?, page_count = ?, metadata_status = ?, metadata_confidence = ?, cover_path = ? WHERE id = ?",
        )
        .bind(&book.title)
        .bind(&book.sort_title)
        .bind(&book.description)
        .bind(&book.language)
        .bind(&pub_date)
        .bind(&publisher_id)
        .bind(&updated_at)
        .bind(book.rating)
        .bind(book.page_count)
        .bind(&status)
        .bind(book.metadata_confidence)
        .bind(&book.cover_path)
        .bind(&id)
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
        let result = sqlx::query("DELETE FROM books WHERE id = ?")
            .bind(&id_str)
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

    pub async fn get_with_relations(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<BookWithRelations, DbError> {
        let book = Self::get_by_id(pool, id).await?;
        let id_str = id.to_string();

        // Fetch authors
        let author_rows = sqlx::query_as::<_, BookAuthorRow>(
            "SELECT a.id, a.name, a.sort_name, ba.role, ba.position FROM authors a JOIN book_authors ba ON ba.author_id = a.id WHERE ba.book_id = ? ORDER BY ba.position",
        )
        .bind(&id_str)
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
        let series_rows = sqlx::query_as::<_, BookSeriesRow>(
            "SELECT s.id, s.name, s.description, bs.position FROM series s JOIN book_series bs ON bs.series_id = s.id WHERE bs.book_id = ? ORDER BY bs.position",
        )
        .bind(&id_str)
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
        let file_rows = sqlx::query_as::<_, BookFileRow>(
            "SELECT id, book_id, format, storage_path, file_size, hash, added_at FROM book_files WHERE book_id = ?",
        )
        .bind(&id_str)
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let files = file_rows
            .into_iter()
            .map(BookFileRow::into_book_file)
            .collect::<Result<Vec<_>, _>>()?;

        // Fetch identifiers
        let ident_rows = sqlx::query_as::<_, IdentifierRow>(
            "SELECT id, book_id, identifier_type, value, source_type, source_name, confidence FROM identifiers WHERE book_id = ?",
        )
        .bind(&id_str)
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let identifiers = ident_rows
            .into_iter()
            .map(IdentifierRow::into_identifier)
            .collect::<Result<Vec<_>, _>>()?;

        // Fetch tags
        let tag_rows = sqlx::query_as::<_, TagRow>(
            "SELECT t.id, t.name, t.category FROM tags t JOIN book_tags bt ON bt.tag_id = t.id WHERE bt.book_id = ?",
        )
        .bind(&id_str)
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let tags = tag_rows
            .into_iter()
            .map(TagRow::into_tag)
            .collect::<Result<Vec<_>, _>>()?;

        // Fetch publisher name
        let publisher_name: Option<String> = if let Some(pid) = book.publisher_id {
            sqlx::query_scalar("SELECT name FROM publishers WHERE id = ?")
                .bind(pid.to_string())
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

    /// Link a book to an author.
    pub async fn add_author(
        pool: &SqlitePool,
        book_id: Uuid,
        author_id: Uuid,
        role: &str,
        position: i32,
    ) -> Result<(), DbError> {
        sqlx::query(
            "INSERT OR IGNORE INTO book_authors (book_id, author_id, role, position) VALUES (?, ?, ?, ?)",
        )
        .bind(book_id.to_string())
        .bind(author_id.to_string())
        .bind(role)
        .bind(position)
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
        sqlx::query(
            "INSERT OR IGNORE INTO book_series (book_id, series_id, position) VALUES (?, ?, ?)",
        )
        .bind(book_id.to_string())
        .bind(series_id.to_string())
        .bind(position)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// Link a book to a tag.
    pub async fn add_tag(pool: &SqlitePool, book_id: Uuid, tag_id: Uuid) -> Result<(), DbError> {
        sqlx::query("INSERT OR IGNORE INTO book_tags (book_id, tag_id) VALUES (?, ?)")
            .bind(book_id.to_string())
            .bind(tag_id.to_string())
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }
}

// ── Row types for sqlx mapping ──────────────────────────────────

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
    page_count: Option<i32>,
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
            page_count: self.page_count,
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
    position: i32,
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
