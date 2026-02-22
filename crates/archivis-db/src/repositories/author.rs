use archivis_core::errors::DbError;
use archivis_core::models::Author;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::types::{PaginatedResult, PaginationParams, SortOrder};

pub struct AuthorRepository;

/// Helper to fetch a page of author rows with a given ORDER BY clause.
macro_rules! fetch_author_rows {
    ($sql:literal, $pool:expr $(, $bind:expr)*) => {
        sqlx::query_as!(AuthorRow, $sql $(, $bind)*)
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
    ) -> Result<PaginatedResult<Author>, DbError> {
        let limit = params.per_page;
        let offset = params.offset();

        let total = sqlx::query_scalar!("SELECT COUNT(*) FROM authors")
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let rows =
            match (params.sort_by.as_str(), params.sort_order) {
                ("name", SortOrder::Asc) => {
                    fetch_author_rows!(
                "SELECT id, name, sort_name FROM authors ORDER BY name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            )
                }
                ("name", SortOrder::Desc) => {
                    fetch_author_rows!(
                "SELECT id, name, sort_name FROM authors ORDER BY name DESC LIMIT ? OFFSET ?",
                pool, limit, offset
            )
                }
                (_, SortOrder::Desc) => fetch_author_rows!(
                "SELECT id, name, sort_name FROM authors ORDER BY sort_name DESC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
                // Default: sort_name ASC
                _ => fetch_author_rows!(
                "SELECT id, name, sort_name FROM authors ORDER BY sort_name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            };

        let authors = rows
            .into_iter()
            .map(AuthorRow::into_author)
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
    ) -> Result<PaginatedResult<Author>, DbError> {
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
            ("name", SortOrder::Asc) => fetch_author_rows!(
                "SELECT id, name, sort_name FROM authors WHERE name LIKE ? COLLATE NOCASE OR sort_name LIKE ? COLLATE NOCASE ORDER BY name ASC LIMIT ? OFFSET ?",
                pool, pattern, pattern, limit, offset
            ),
            ("name", SortOrder::Desc) => fetch_author_rows!(
                "SELECT id, name, sort_name FROM authors WHERE name LIKE ? COLLATE NOCASE OR sort_name LIKE ? COLLATE NOCASE ORDER BY name DESC LIMIT ? OFFSET ?",
                pool, pattern, pattern, limit, offset
            ),
            (_, SortOrder::Desc) => fetch_author_rows!(
                "SELECT id, name, sort_name FROM authors WHERE name LIKE ? COLLATE NOCASE OR sort_name LIKE ? COLLATE NOCASE ORDER BY sort_name DESC LIMIT ? OFFSET ?",
                pool, pattern, pattern, limit, offset
            ),
            _ => fetch_author_rows!(
                "SELECT id, name, sort_name FROM authors WHERE name LIKE ? COLLATE NOCASE OR sort_name LIKE ? COLLATE NOCASE ORDER BY sort_name ASC LIMIT ? OFFSET ?",
                pool, pattern, pattern, limit, offset
            ),
        };

        let authors = rows
            .into_iter()
            .map(AuthorRow::into_author)
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
