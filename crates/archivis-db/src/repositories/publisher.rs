use archivis_core::errors::DbError;
use archivis_core::models::Publisher;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

use super::types::{PaginatedResult, PaginationParams, SortOrder};

pub struct PublisherRepository;

/// Helper to fetch a page of publisher rows with a given ORDER BY clause.
macro_rules! fetch_publisher_rows {
    ($sql:literal, $pool:expr $(, $bind:expr)*) => {
        sqlx::query_as!(PublisherRow, $sql $(, $bind)*)
            .fetch_all($pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?
    };
}

impl PublisherRepository {
    pub async fn create(pool: &SqlitePool, publisher: &Publisher) -> Result<(), DbError> {
        let id = publisher.id.to_string();
        sqlx::query!(
            "INSERT INTO publishers (id, name) VALUES (?, ?)",
            id,
            publisher.name,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Publisher, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            PublisherRow,
            "SELECT id, name FROM publishers WHERE id = ?",
            id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "publisher",
            id: id_str,
        })?;

        row.into_publisher()
    }

    pub async fn list(
        pool: &SqlitePool,
        params: &PaginationParams,
    ) -> Result<PaginatedResult<Publisher>, DbError> {
        let limit = params.per_page;
        let offset = params.offset();

        let total = sqlx::query_scalar!("SELECT COUNT(*) FROM publishers")
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let rows = match params.sort_order {
            SortOrder::Desc => fetch_publisher_rows!(
                "SELECT id, name FROM publishers ORDER BY name DESC LIMIT ? OFFSET ?",
                pool,
                limit,
                offset
            ),
            SortOrder::Asc => fetch_publisher_rows!(
                "SELECT id, name FROM publishers ORDER BY name ASC LIMIT ? OFFSET ?",
                pool,
                limit,
                offset
            ),
        };

        let items = rows
            .into_iter()
            .map(PublisherRow::into_publisher)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(items, total as u32, params))
    }

    /// Search publishers by name (case-insensitive substring match).
    pub async fn search(
        pool: &SqlitePool,
        query: &str,
        params: &PaginationParams,
    ) -> Result<PaginatedResult<Publisher>, DbError> {
        let limit = params.per_page;
        let offset = params.offset();
        let pattern = format!("%{query}%");

        let total = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM publishers WHERE name LIKE ? COLLATE NOCASE",
            pattern,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let rows = match params.sort_order {
            SortOrder::Desc => fetch_publisher_rows!(
                "SELECT id, name FROM publishers WHERE name LIKE ? COLLATE NOCASE ORDER BY name DESC LIMIT ? OFFSET ?",
                pool, pattern, limit, offset
            ),
            SortOrder::Asc => fetch_publisher_rows!(
                "SELECT id, name FROM publishers WHERE name LIKE ? COLLATE NOCASE ORDER BY name ASC LIMIT ? OFFSET ?",
                pool, pattern, limit, offset
            ),
        };

        let items = rows
            .into_iter()
            .map(PublisherRow::into_publisher)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(items, total as u32, params))
    }

    pub async fn update(pool: &SqlitePool, publisher: &Publisher) -> Result<(), DbError> {
        let id = publisher.id.to_string();
        let result = sqlx::query!(
            "UPDATE publishers SET name = ? WHERE id = ?",
            publisher.name,
            id,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "publisher",
                id,
            });
        }

        Ok(())
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query!("DELETE FROM publishers WHERE id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "publisher",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Find a publisher by name (case-insensitive exact match), or create it if it doesn't exist.
    pub async fn find_or_create(pool: &SqlitePool, name: &str) -> Result<Publisher, DbError> {
        if let Some(existing) = Self::find_by_name(pool, name).await? {
            return Ok(existing);
        }

        let publisher = Publisher::new(name);
        Self::create(pool, &publisher).await?;
        Ok(publisher)
    }

    pub async fn find_by_name(pool: &SqlitePool, name: &str) -> Result<Option<Publisher>, DbError> {
        let row = sqlx::query_as!(
            PublisherRow,
            "SELECT id, name FROM publishers WHERE name = ? COLLATE NOCASE",
            name,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        row.map(PublisherRow::into_publisher).transpose()
    }

    pub async fn find_by_name_conn(
        conn: &mut SqliteConnection,
        name: &str,
    ) -> Result<Option<Publisher>, DbError> {
        let row = sqlx::query_as!(
            PublisherRow,
            "SELECT id, name FROM publishers WHERE name = ? COLLATE NOCASE",
            name,
        )
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        row.map(PublisherRow::into_publisher).transpose()
    }

    pub async fn create_conn(
        conn: &mut SqliteConnection,
        publisher: &Publisher,
    ) -> Result<(), DbError> {
        let id = publisher.id.to_string();
        sqlx::query!(
            "INSERT INTO publishers (id, name) VALUES (?, ?)",
            id,
            publisher.name,
        )
        .execute(conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// Count books referencing this publisher.
    #[allow(clippy::cast_possible_truncation)]
    pub async fn count_books(pool: &SqlitePool, publisher_id: Uuid) -> Result<i32, DbError> {
        let id_str = publisher_id.to_string();
        let count =
            sqlx::query_scalar!("SELECT COUNT(*) FROM books WHERE publisher_id = ?", id_str,)
                .fetch_one(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(count as i32)
    }
}

#[derive(sqlx::FromRow)]
struct PublisherRow {
    id: String,
    name: String,
}

impl PublisherRow {
    fn into_publisher(self) -> Result<Publisher, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid publisher UUID: {e}")))?;
        Ok(Publisher {
            id,
            name: self.name,
        })
    }
}
