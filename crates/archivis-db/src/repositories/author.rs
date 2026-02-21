use archivis_core::errors::DbError;
use archivis_core::models::Author;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::types::{PaginatedResult, PaginationParams};

pub struct AuthorRepository;

impl AuthorRepository {
    pub async fn create(pool: &SqlitePool, author: &Author) -> Result<(), DbError> {
        let id = author.id.to_string();
        sqlx::query("INSERT INTO authors (id, name, sort_name) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(&author.name)
            .bind(&author.sort_name)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Author, DbError> {
        let id_str = id.to_string();
        let row =
            sqlx::query_as::<_, AuthorRow>("SELECT id, name, sort_name FROM authors WHERE id = ?")
                .bind(&id_str)
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
        let sort_col = match params.sort_by.as_str() {
            "name" => "name",
            _ => "sort_name",
        };
        let sort_dir = params.sort_order.as_sql();
        let limit = params.per_page;
        let offset = params.offset();

        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM authors")
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let sql = format!(
            "SELECT id, name, sort_name FROM authors ORDER BY {sort_col} {sort_dir} LIMIT {limit} OFFSET {offset}"
        );

        let rows = sqlx::query_as::<_, AuthorRow>(&sql)
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let authors = rows
            .into_iter()
            .map(AuthorRow::into_author)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(authors, total as u32, params))
    }

    pub async fn update(pool: &SqlitePool, author: &Author) -> Result<(), DbError> {
        let id = author.id.to_string();
        let result = sqlx::query("UPDATE authors SET name = ?, sort_name = ? WHERE id = ?")
            .bind(&author.name)
            .bind(&author.sort_name)
            .bind(&id)
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
        let result = sqlx::query("DELETE FROM authors WHERE id = ?")
            .bind(&id_str)
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
        let sort_col = match params.sort_by.as_str() {
            "name" => "name",
            _ => "sort_name",
        };
        let sort_dir = params.sort_order.as_sql();
        let limit = params.per_page;
        let offset = params.offset();
        let pattern = format!("%{query}%");

        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM authors WHERE name LIKE ? COLLATE NOCASE OR sort_name LIKE ? COLLATE NOCASE",
        )
        .bind(&pattern)
        .bind(&pattern)
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let sql = format!(
            "SELECT id, name, sort_name FROM authors WHERE name LIKE ? COLLATE NOCASE OR sort_name LIKE ? COLLATE NOCASE ORDER BY {sort_col} {sort_dir} LIMIT {limit} OFFSET {offset}"
        );

        let rows = sqlx::query_as::<_, AuthorRow>(&sql)
            .bind(&pattern)
            .bind(&pattern)
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let authors = rows
            .into_iter()
            .map(AuthorRow::into_author)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(authors, total as u32, params))
    }

    pub async fn find_by_name(pool: &SqlitePool, name: &str) -> Result<Option<Author>, DbError> {
        let row = sqlx::query_as::<_, AuthorRow>(
            "SELECT id, name, sort_name FROM authors WHERE name = ? COLLATE NOCASE",
        )
        .bind(name)
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
