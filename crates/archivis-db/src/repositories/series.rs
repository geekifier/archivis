use archivis_core::errors::DbError;
use archivis_core::models::Series;
use sqlx::{SqliteConnection, SqlitePool};
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

    pub async fn create_conn(conn: &mut SqliteConnection, series: &Series) -> Result<(), DbError> {
        let id = series.id.to_string();
        sqlx::query!(
            "INSERT INTO series (id, name, description) VALUES (?, ?, ?)",
            id,
            series.name,
            series.description,
        )
        .execute(conn)
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

    pub async fn find_or_create_conn(
        conn: &mut SqliteConnection,
        name: &str,
    ) -> Result<Series, DbError> {
        let row = sqlx::query_as!(
            SeriesRow,
            "SELECT id, name, description FROM series WHERE name = ? COLLATE NOCASE",
            name,
        )
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if let Some(row) = row {
            return row.into_series();
        }

        let series = Series::new(name);
        Self::create_conn(conn, &series).await?;
        Ok(series)
    }

    /// Find a series by name (case-insensitive exact match).
    pub async fn find_by_name(pool: &SqlitePool, name: &str) -> Result<Option<Series>, DbError> {
        let row = sqlx::query_as!(
            SeriesRow,
            "SELECT id, name, description FROM series WHERE name = ? COLLATE NOCASE",
            name,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        row.map(SeriesRow::into_series).transpose()
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

    /// Fetch a single series along with its `book_count`.
    pub async fn get_by_id_with_count(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<SeriesWithBookCount, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            SeriesWithCountRow,
            r#"SELECT s.id, s.name, s.description,
                (SELECT COUNT(*) FROM book_series bs WHERE bs.series_id = s.id) AS "book_count!: i64"
             FROM series s WHERE s.id = ?"#,
            id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "series",
            id: id_str,
        })?;

        row.into_series_with_count()
    }

    /// Merge `source_ids` into `target_id` in a single transaction.
    ///
    /// Rewrites `book_series` rows via DELETE+INSERT (not UPDATE) so the existing
    /// `book_series_fts_insert`/`book_series_fts_delete` triggers fire and
    /// `books_fts.series_names` stays correct. The schema has no UPDATE trigger
    /// on `book_series`, so an `UPDATE … SET series_id = …` would silently leave
    /// FTS stale.
    ///
    /// If `new_name` is provided, the target series is renamed in the same
    /// transaction. The `series_fts_update` trigger refreshes FTS for every book
    /// in the renamed series automatically.
    pub async fn merge(
        pool: &SqlitePool,
        target_id: Uuid,
        source_ids: &[Uuid],
        new_name: Option<&str>,
    ) -> Result<(), DbError> {
        let target_str = target_id.to_string();

        let mut tx = pool
            .begin()
            .await
            .map_err(|e| DbError::Transaction(e.to_string()))?;

        // 1. Verify the target exists. If a rename was requested, fold the
        //    existence check into the UPDATE; otherwise do a cheap probe. The
        //    series UPDATE trigger rebuilds books_fts for every book in the
        //    renamed series.
        if let Some(name) = new_name {
            let result = sqlx::query!("UPDATE series SET name = ? WHERE id = ?", name, target_str,)
                .execute(tx.as_mut())
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;
            if result.rows_affected() == 0 {
                return Err(DbError::NotFound {
                    entity: "series",
                    id: target_str,
                });
            }
        } else {
            let exists = sqlx::query_scalar!(
                r#"SELECT 1 AS "exists!: i64" FROM series WHERE id = ?"#,
                target_str,
            )
            .fetch_optional(tx.as_mut())
            .await
            .map_err(|e| DbError::Query(e.to_string()))?
            .is_some();
            if !exists {
                return Err(DbError::NotFound {
                    entity: "series",
                    id: target_str,
                });
            }
        }

        // 2. For each source: move books to target, then delete the source row.
        //    Source existence is verified by the rows_affected check on the
        //    series DELETE — the INSERT OR IGNORE and book_series DELETE are
        //    intentional no-ops if the source has no rows.
        for source_id in source_ids {
            let source_str = source_id.to_string();

            // Move books from source to target via INSERT OR IGNORE + DELETE so
            // the existing book_series FTS triggers fire. PK conflict on
            // (book_id, series_id) is resolved by INSERT OR IGNORE — the
            // target's existing row wins, preserving its position.
            sqlx::query!(
                "INSERT OR IGNORE INTO book_series (book_id, series_id, position) \
                 SELECT book_id, ?, position FROM book_series WHERE series_id = ?",
                target_str,
                source_str,
            )
            .execute(tx.as_mut())
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

            sqlx::query!("DELETE FROM book_series WHERE series_id = ?", source_str,)
                .execute(tx.as_mut())
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;

            let result = sqlx::query!("DELETE FROM series WHERE id = ?", source_str)
                .execute(tx.as_mut())
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;
            if result.rows_affected() == 0 {
                return Err(DbError::NotFound {
                    entity: "series",
                    id: source_str,
                });
            }
        }

        tx.commit()
            .await
            .map_err(|e| DbError::Transaction(e.to_string()))?;
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
            #[allow(clippy::cast_possible_truncation)]
            book_count: self.book_count as u32,
        })
    }
}
