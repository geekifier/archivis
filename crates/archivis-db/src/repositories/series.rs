use archivis_core::errors::DbError;
use archivis_core::models::Series;
use sqlx::SqlitePool;
use uuid::Uuid;

use super::types::{PaginatedResult, PaginationParams};

pub struct SeriesRepository;

impl SeriesRepository {
    pub async fn create(pool: &SqlitePool, series: &Series) -> Result<(), DbError> {
        let id = series.id.to_string();
        sqlx::query("INSERT INTO series (id, name, description) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(&series.name)
            .bind(&series.description)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Series, DbError> {
        let id_str = id.to_string();
        let row =
            sqlx::query_as::<_, SeriesRow>("SELECT id, name, description FROM series WHERE id = ?")
                .bind(&id_str)
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
        let sort_col = "name";
        let sort_dir = params.sort_order.as_sql();
        let limit = params.per_page;
        let offset = params.offset();

        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM series")
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let sql = format!(
            "SELECT id, name, description FROM series ORDER BY {sort_col} {sort_dir} LIMIT {limit} OFFSET {offset}"
        );

        let rows = sqlx::query_as::<_, SeriesRow>(&sql)
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        let items = rows
            .into_iter()
            .map(SeriesRow::into_series)
            .collect::<Result<Vec<_>, _>>()?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(PaginatedResult::new(items, total as u32, params))
    }

    pub async fn update(pool: &SqlitePool, series: &Series) -> Result<(), DbError> {
        let id = series.id.to_string();
        let result = sqlx::query("UPDATE series SET name = ?, description = ? WHERE id = ?")
            .bind(&series.name)
            .bind(&series.description)
            .bind(&id)
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

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query("DELETE FROM series WHERE id = ?")
            .bind(&id_str)
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
