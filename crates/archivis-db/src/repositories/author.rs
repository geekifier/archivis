use archivis_core::errors::DbError;
use archivis_core::models::Author;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

use super::types::{PaginatedResult, PaginationParams, SortOrder};

pub struct AuthorRepository;

/// An author together with a pre-computed book count (from list/search queries).
pub struct AuthorWithBookCount {
    pub author: Author,
    pub book_count: u32,
}

/// Helper to fetch a page of author-with-count rows with a given ORDER BY clause.
macro_rules! fetch_author_count_rows {
    ($sql:literal, $pool:expr $(, $bind:expr)*) => {
        sqlx::query_as!(AuthorWithCountRow, $sql $(, $bind)*)
            .fetch_all($pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?
    };
}

impl AuthorRepository {
    pub async fn create(pool: &SqlitePool, author: &Author) -> Result<(), DbError> {
        let id = author.id.to_string();
        sqlx::query!(
            "INSERT INTO authors (id, name, sort_name) VALUES (?, ?, ?)",
            id,
            author.name,
            author.sort_name,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn create_conn(conn: &mut SqliteConnection, author: &Author) -> Result<(), DbError> {
        let id = author.id.to_string();
        sqlx::query!(
            "INSERT INTO authors (id, name, sort_name) VALUES (?, ?, ?)",
            id,
            author.name,
            author.sort_name,
        )
        .execute(conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Author, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            AuthorRow,
            "SELECT id, name, sort_name FROM authors WHERE id = ?",
            id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "author",
            id: id_str,
        })?;

        row.into_author()
    }

    pub async fn list(
        pool: &SqlitePool,
        params: &PaginationParams,
    ) -> Result<PaginatedResult<AuthorWithBookCount>, DbError> {
        let limit = params.per_page;
        let offset = params.offset();

        let total = sqlx::query_scalar!("SELECT COUNT(*) FROM authors")
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let rows = match (params.sort_by.as_str(), params.sort_order) {
            ("name", SortOrder::Asc) => fetch_author_count_rows!(
                "SELECT a.id, a.name, a.sort_name, (SELECT COUNT(*) FROM book_authors ba WHERE ba.author_id = a.id) AS book_count FROM authors a ORDER BY a.name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            ("name", SortOrder::Desc) => fetch_author_count_rows!(
                "SELECT a.id, a.name, a.sort_name, (SELECT COUNT(*) FROM book_authors ba WHERE ba.author_id = a.id) AS book_count FROM authors a ORDER BY a.name DESC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            ("book_count", SortOrder::Asc) => fetch_author_count_rows!(
                "SELECT a.id, a.name, a.sort_name, (SELECT COUNT(*) FROM book_authors ba WHERE ba.author_id = a.id) AS book_count FROM authors a ORDER BY book_count ASC, a.sort_name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            ("book_count", SortOrder::Desc) => fetch_author_count_rows!(
                "SELECT a.id, a.name, a.sort_name, (SELECT COUNT(*) FROM book_authors ba WHERE ba.author_id = a.id) AS book_count FROM authors a ORDER BY book_count DESC, a.sort_name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            (_, SortOrder::Desc) => fetch_author_count_rows!(
                "SELECT a.id, a.name, a.sort_name, (SELECT COUNT(*) FROM book_authors ba WHERE ba.author_id = a.id) AS book_count FROM authors a ORDER BY a.sort_name DESC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            // Default: sort_name ASC
            _ => fetch_author_count_rows!(
                "SELECT a.id, a.name, a.sort_name, (SELECT COUNT(*) FROM book_authors ba WHERE ba.author_id = a.id) AS book_count FROM authors a ORDER BY a.sort_name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
        };

        let authors = rows
            .into_iter()
            .map(AuthorWithCountRow::into_author_with_count)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(authors, total as u32, params))
    }

    pub async fn update(pool: &SqlitePool, author: &Author) -> Result<(), DbError> {
        let id = author.id.to_string();
        let result = sqlx::query!(
            "UPDATE authors SET name = ?, sort_name = ? WHERE id = ?",
            author.name,
            author.sort_name,
            id,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "author",
                id,
            });
        }

        Ok(())
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query!("DELETE FROM authors WHERE id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "author",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Search authors by name (case-insensitive substring match).
    pub async fn search(
        pool: &SqlitePool,
        query: &str,
        params: &PaginationParams,
    ) -> Result<PaginatedResult<AuthorWithBookCount>, DbError> {
        let limit = params.per_page;
        let offset = params.offset();
        let pattern = format!("%{query}%");

        let total = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM authors WHERE name LIKE ? COLLATE NOCASE OR sort_name LIKE ? COLLATE NOCASE",
            pattern,
            pattern,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let rows = match (params.sort_by.as_str(), params.sort_order) {
            ("name", SortOrder::Asc) => fetch_author_count_rows!(
                "SELECT a.id, a.name, a.sort_name, (SELECT COUNT(*) FROM book_authors ba WHERE ba.author_id = a.id) AS book_count FROM authors a WHERE a.name LIKE ? COLLATE NOCASE OR a.sort_name LIKE ? COLLATE NOCASE ORDER BY a.name ASC LIMIT ? OFFSET ?",
                pool, pattern, pattern, limit, offset
            ),
            ("name", SortOrder::Desc) => fetch_author_count_rows!(
                "SELECT a.id, a.name, a.sort_name, (SELECT COUNT(*) FROM book_authors ba WHERE ba.author_id = a.id) AS book_count FROM authors a WHERE a.name LIKE ? COLLATE NOCASE OR a.sort_name LIKE ? COLLATE NOCASE ORDER BY a.name DESC LIMIT ? OFFSET ?",
                pool, pattern, pattern, limit, offset
            ),
            ("book_count", SortOrder::Asc) => fetch_author_count_rows!(
                "SELECT a.id, a.name, a.sort_name, (SELECT COUNT(*) FROM book_authors ba WHERE ba.author_id = a.id) AS book_count FROM authors a WHERE a.name LIKE ? COLLATE NOCASE OR a.sort_name LIKE ? COLLATE NOCASE ORDER BY book_count ASC, a.sort_name ASC LIMIT ? OFFSET ?",
                pool, pattern, pattern, limit, offset
            ),
            ("book_count", SortOrder::Desc) => fetch_author_count_rows!(
                "SELECT a.id, a.name, a.sort_name, (SELECT COUNT(*) FROM book_authors ba WHERE ba.author_id = a.id) AS book_count FROM authors a WHERE a.name LIKE ? COLLATE NOCASE OR a.sort_name LIKE ? COLLATE NOCASE ORDER BY book_count DESC, a.sort_name ASC LIMIT ? OFFSET ?",
                pool, pattern, pattern, limit, offset
            ),
            (_, SortOrder::Desc) => fetch_author_count_rows!(
                "SELECT a.id, a.name, a.sort_name, (SELECT COUNT(*) FROM book_authors ba WHERE ba.author_id = a.id) AS book_count FROM authors a WHERE a.name LIKE ? COLLATE NOCASE OR a.sort_name LIKE ? COLLATE NOCASE ORDER BY a.sort_name DESC LIMIT ? OFFSET ?",
                pool, pattern, pattern, limit, offset
            ),
            _ => fetch_author_count_rows!(
                "SELECT a.id, a.name, a.sort_name, (SELECT COUNT(*) FROM book_authors ba WHERE ba.author_id = a.id) AS book_count FROM authors a WHERE a.name LIKE ? COLLATE NOCASE OR a.sort_name LIKE ? COLLATE NOCASE ORDER BY a.sort_name ASC LIMIT ? OFFSET ?",
                pool, pattern, pattern, limit, offset
            ),
        };

        let authors = rows
            .into_iter()
            .map(AuthorWithCountRow::into_author_with_count)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(authors, total as u32, params))
    }

    pub async fn find_by_name(pool: &SqlitePool, name: &str) -> Result<Option<Author>, DbError> {
        let row = sqlx::query_as!(
            AuthorRow,
            "SELECT id, name, sort_name FROM authors WHERE name = ? COLLATE NOCASE",
            name,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        row.map(AuthorRow::into_author).transpose()
    }

    pub async fn find_by_name_conn(
        conn: &mut SqliteConnection,
        name: &str,
    ) -> Result<Option<Author>, DbError> {
        let row = sqlx::query_as!(
            AuthorRow,
            "SELECT id, name, sort_name FROM authors WHERE name = ? COLLATE NOCASE",
            name,
        )
        .fetch_optional(conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        row.map(AuthorRow::into_author).transpose()
    }
}

#[derive(sqlx::FromRow)]
struct AuthorRow {
    id: String,
    name: String,
    sort_name: String,
}

impl AuthorRow {
    fn into_author(self) -> Result<Author, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid author UUID: {e}")))?;
        Ok(Author {
            id,
            name: self.name,
            sort_name: self.sort_name,
        })
    }
}

#[derive(sqlx::FromRow)]
struct AuthorWithCountRow {
    id: String,
    name: String,
    sort_name: String,
    book_count: i64,
}

impl AuthorWithCountRow {
    fn into_author_with_count(self) -> Result<AuthorWithBookCount, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid author UUID: {e}")))?;
        #[allow(clippy::cast_sign_loss)]
        Ok(AuthorWithBookCount {
            author: Author {
                id,
                name: self.name,
                sort_name: self.sort_name,
            },
            #[allow(clippy::cast_possible_truncation)]
            book_count: self.book_count as u32,
        })
    }
}
