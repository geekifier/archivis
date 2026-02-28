use archivis_core::errors::DbError;
use archivis_core::models::Series;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::types::{PaginatedResult, PaginationParams, SortOrder};

pub struct SeriesRepository;

/// A series together with a pre-computed book count (from list/search queries).
pub struct SeriesWithBookCount {
    pub series: Series,
    pub book_count: u32,
}

/// Helper to fetch a page of series-with-count rows with a given ORDER BY clause.
macro_rules! fetch_series_count_rows {
    ($sql:literal, $pool:expr $(, $bind:expr)*) => {
        sqlx::query_as!(SeriesWithCountRow, $sql $(, $bind)*)
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
    ) -> Result<PaginatedResult<SeriesWithBookCount>, DbError> {
        let limit = params.per_page;
        let offset = params.offset();

        let total = sqlx::query_scalar!("SELECT COUNT(*) FROM series")
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let rows = match (params.sort_by.as_str(), params.sort_order) {
            ("book_count", SortOrder::Asc) => fetch_series_count_rows!(
                "SELECT s.id, s.name, s.description, (SELECT COUNT(*) FROM book_series bs WHERE bs.series_id = s.id) AS book_count FROM series s ORDER BY book_count ASC, s.name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            ("book_count", SortOrder::Desc) => fetch_series_count_rows!(
                "SELECT s.id, s.name, s.description, (SELECT COUNT(*) FROM book_series bs WHERE bs.series_id = s.id) AS book_count FROM series s ORDER BY book_count DESC, s.name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            (_, SortOrder::Desc) => fetch_series_count_rows!(
                "SELECT s.id, s.name, s.description, (SELECT COUNT(*) FROM book_series bs WHERE bs.series_id = s.id) AS book_count FROM series s ORDER BY s.name DESC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
            _ => fetch_series_count_rows!(
                "SELECT s.id, s.name, s.description, (SELECT COUNT(*) FROM book_series bs WHERE bs.series_id = s.id) AS book_count FROM series s ORDER BY s.name ASC LIMIT ? OFFSET ?",
                pool, limit, offset
            ),
        };

        let items = rows
            .into_iter()
            .map(SeriesWithCountRow::into_series_with_count)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(items, total as u32, params))
    }

    /// Search series by name (case-insensitive substring match).
    pub async fn search(
        pool: &SqlitePool,
        query: &str,
        params: &PaginationParams,
    ) -> Result<PaginatedResult<SeriesWithBookCount>, DbError> {
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

        let rows = match (params.sort_by.as_str(), params.sort_order) {
            ("book_count", SortOrder::Asc) => fetch_series_count_rows!(
                "SELECT s.id, s.name, s.description, (SELECT COUNT(*) FROM book_series bs WHERE bs.series_id = s.id) AS book_count FROM series s WHERE s.name LIKE ? COLLATE NOCASE ORDER BY book_count ASC, s.name ASC LIMIT ? OFFSET ?",
                pool, pattern, limit, offset
            ),
            ("book_count", SortOrder::Desc) => fetch_series_count_rows!(
                "SELECT s.id, s.name, s.description, (SELECT COUNT(*) FROM book_series bs WHERE bs.series_id = s.id) AS book_count FROM series s WHERE s.name LIKE ? COLLATE NOCASE ORDER BY book_count DESC, s.name ASC LIMIT ? OFFSET ?",
                pool, pattern, limit, offset
            ),
            (_, SortOrder::Desc) => fetch_series_count_rows!(
                "SELECT s.id, s.name, s.description, (SELECT COUNT(*) FROM book_series bs WHERE bs.series_id = s.id) AS book_count FROM series s WHERE s.name LIKE ? COLLATE NOCASE ORDER BY s.name DESC LIMIT ? OFFSET ?",
                pool, pattern, limit, offset
            ),
            _ => fetch_series_count_rows!(
                "SELECT s.id, s.name, s.description, (SELECT COUNT(*) FROM book_series bs WHERE bs.series_id = s.id) AS book_count FROM series s WHERE s.name LIKE ? COLLATE NOCASE ORDER BY s.name ASC LIMIT ? OFFSET ?",
                pool, pattern, limit, offset
            ),
        };

        let items = rows
            .into_iter()
            .map(SeriesWithCountRow::into_series_with_count)
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

#[derive(sqlx::FromRow)]
struct SeriesWithCountRow {
    id: String,
    name: String,
    description: Option<String>,
    book_count: i64,
}

impl SeriesWithCountRow {
    fn into_series_with_count(self) -> Result<SeriesWithBookCount, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid series UUID: {e}")))?;
        #[allow(clippy::cast_sign_loss)]
        Ok(SeriesWithBookCount {
            series: Series {
                id,
                name: self.name,
                description: self.description,
            },
            book_count: self.book_count as u32,
        })
    }
}
