use archivis_core::errors::DbError;
use archivis_core::models::Series;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::types::{PaginatedResult, PaginationParams, SortOrder};

pub struct SeriesRepository;

/// Helper to fetch a page of series rows with a given ORDER BY clause.
macro_rules! fetch_series_rows {
    ($sql:literal, $pool:expr $(, $bind:expr)*) => {
        sqlx::query_as!(SeriesRow, $sql $(, $bind)*)
            .fetch_all($pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?
    };
}

impl SeriesRepository {
    pub async fn create(pool: &SqlitePool, series: &Series) -> Result<(), DbError> {
        let id = series.id.to_string();
        sqlx::query!(
            "INSERT INTO series (id, name, description) VALUES (?, ?, ?)",
            id,
            series.name,
            series.description,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Series, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            SeriesRow,
            "SELECT id, name, description FROM series WHERE id = ?",
            id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "series",
            id: id_str,
        })?;

        row.into_series()
    }

    pub async fn list(
        pool: &SqlitePool,
        params: &PaginationParams,
    ) -> Result<PaginatedResult<Series>, DbError> {
        let limit = params.per_page;
        let offset = params.offset();

        let total = sqlx::query_scalar!("SELECT COUNT(*) FROM series")
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let rows = match params.sort_order {
            SortOrder::Desc => fetch_series_rows!(
                "SELECT id, name, description FROM series ORDER BY name DESC LIMIT ? OFFSET ?",
                pool,
                limit,
                offset
            ),
            SortOrder::Asc => fetch_series_rows!(
                "SELECT id, name, description FROM series ORDER BY name ASC LIMIT ? OFFSET ?",
                pool,
                limit,
                offset
            ),
        };

        let items = rows
            .into_iter()
            .map(SeriesRow::into_series)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(items, total as u32, params))
    }

    /// Search series by name (case-insensitive substring match).
    pub async fn search(
        pool: &SqlitePool,
        query: &str,
        params: &PaginationParams,
    ) -> Result<PaginatedResult<Series>, DbError> {
        let limit = params.per_page;
        let offset = params.offset();
        let pattern = format!("%{query}%");

        let total = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM series WHERE name LIKE ? COLLATE NOCASE",
            pattern,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        let rows = match params.sort_order {
            SortOrder::Desc => fetch_series_rows!(
                "SELECT id, name, description FROM series WHERE name LIKE ? COLLATE NOCASE ORDER BY name DESC LIMIT ? OFFSET ?",
                pool, pattern, limit, offset
            ),
            SortOrder::Asc => fetch_series_rows!(
                "SELECT id, name, description FROM series WHERE name LIKE ? COLLATE NOCASE ORDER BY name ASC LIMIT ? OFFSET ?",
                pool, pattern, limit, offset
            ),
        };

        let items = rows
            .into_iter()
            .map(SeriesRow::into_series)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(items, total as u32, params))
    }

    pub async fn update(pool: &SqlitePool, series: &Series) -> Result<(), DbError> {
        let id = series.id.to_string();
        let result = sqlx::query!(
            "UPDATE series SET name = ?, description = ? WHERE id = ?",
            series.name,
            series.description,
            id,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "series",
                id,
            });
        }

        Ok(())
    }

    /// Find a series by name (case-insensitive exact match), or create it if it doesn't exist.
    pub async fn find_or_create(pool: &SqlitePool, name: &str) -> Result<Series, DbError> {
        let row = sqlx::query_as!(
            SeriesRow,
            "SELECT id, name, description FROM series WHERE name = ? COLLATE NOCASE",
            name,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if let Some(row) = row {
            return row.into_series();
        }

        let series = Series::new(name);
        Self::create(pool, &series).await?;
        Ok(series)
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query!("DELETE FROM series WHERE id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "series",
                id: id_str,
            });
        }

        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct SeriesRow {
    id: String,
    name: String,
    description: Option<String>,
}

impl SeriesRow {
    fn into_series(self) -> Result<Series, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid series UUID: {e}")))?;
        Ok(Series {
            id,
            name: self.name,
            description: self.description,
        })
    }
}
