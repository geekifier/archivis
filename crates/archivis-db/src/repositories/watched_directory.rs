use archivis_core::errors::DbError;
use archivis_core::models::{WatchMode, WatchedDirectory};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

/// Row type for `sqlx::query_as!` mapping from the `watched_directories` table.
struct WatchedDirectoryRow {
    id: String,
    path: String,
    watch_mode: String,
    poll_interval_secs: Option<i64>,
    enabled: i64,
    last_error: Option<String>,
    created_at: String,
    updated_at: String,
}

impl WatchedDirectoryRow {
    fn into_model(self) -> Result<WatchedDirectory, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid watched_directory UUID: {e}")))?;
        let watch_mode: WatchMode = self
            .watch_mode
            .parse()
            .map_err(|e: String| DbError::Query(e))?;
        let created_at = parse_datetime(&self.created_at, "created_at")?;
        let updated_at = parse_datetime(&self.updated_at, "updated_at")?;

        Ok(WatchedDirectory {
            id,
            path: self.path,
            watch_mode,
            poll_interval_secs: self.poll_interval_secs,
            enabled: self.enabled != 0,
            last_error: self.last_error,
            created_at,
            updated_at,
        })
    }
}

/// Parse an ISO 8601 datetime string, handling both RFC 3339 and `SQLite` default formats.
fn parse_datetime(s: &str, field: &str) -> Result<DateTime<Utc>, DbError> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ")
                .map(|ndt| ndt.and_utc())
        })
        .map_err(|e| DbError::Query(format!("invalid {field}: {e}")))
}

pub struct WatchedDirectoryRepository;

impl WatchedDirectoryRepository {
    pub async fn create(
        pool: &SqlitePool,
        path: &str,
        watch_mode: WatchMode,
        poll_interval_secs: Option<i64>,
    ) -> Result<WatchedDirectory, DbError> {
        let id = Uuid::new_v4();
        let id_str = id.to_string();
        let mode_str = watch_mode.to_string().to_lowercase();

        sqlx::query!(
            "INSERT INTO watched_directories (id, path, watch_mode, poll_interval_secs) VALUES (?, ?, ?, ?)",
            id_str,
            path,
            mode_str,
            poll_interval_secs,
        )
        .execute(pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                DbError::Constraint(format!("watched directory path already exists: {path}"))
            } else {
                DbError::Query(e.to_string())
            }
        })?;

        Self::get_by_id(pool, id).await?.ok_or(DbError::NotFound {
            entity: "watched_directory",
            id: id_str,
        })
    }

    pub async fn get_by_id(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<WatchedDirectory>, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            WatchedDirectoryRow,
            "SELECT id, path, watch_mode, poll_interval_secs, enabled, last_error, created_at, updated_at FROM watched_directories WHERE id = ?",
            id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        row.map(WatchedDirectoryRow::into_model).transpose()
    }

    pub async fn list_enabled(pool: &SqlitePool) -> Result<Vec<WatchedDirectory>, DbError> {
        let rows = sqlx::query_as!(
            WatchedDirectoryRow,
            "SELECT id, path, watch_mode, poll_interval_secs, enabled, last_error, created_at, updated_at FROM watched_directories WHERE enabled = 1 ORDER BY path",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(WatchedDirectoryRow::into_model)
            .collect()
    }

    pub async fn list_all(pool: &SqlitePool) -> Result<Vec<WatchedDirectory>, DbError> {
        let rows = sqlx::query_as!(
            WatchedDirectoryRow,
            "SELECT id, path, watch_mode, poll_interval_secs, enabled, last_error, created_at, updated_at FROM watched_directories ORDER BY path",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(WatchedDirectoryRow::into_model)
            .collect()
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        watch_mode: Option<WatchMode>,
        poll_interval_secs: Option<Option<i64>>,
        enabled: Option<bool>,
    ) -> Result<WatchedDirectory, DbError> {
        let id_str = id.to_string();
        let mode_str = watch_mode.map(|m| m.to_string().to_lowercase());
        let enabled_int = enabled.map(i64::from);

        // Use COALESCE to only update provided fields.
        // For poll_interval_secs, the outer Option indicates "was provided" and the inner
        // Option indicates the actual value (NULL = use global default).
        // We handle this with a separate flag parameter.
        let poll_provided = poll_interval_secs.is_some();
        let poll_value = poll_interval_secs.flatten();

        let result = sqlx::query!(
            "UPDATE watched_directories SET
                watch_mode = COALESCE(?, watch_mode),
                poll_interval_secs = CASE WHEN ? THEN ? ELSE poll_interval_secs END,
                enabled = COALESCE(?, enabled)
            WHERE id = ?",
            mode_str,
            poll_provided,
            poll_value,
            enabled_int,
            id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "watched_directory",
                id: id_str,
            });
        }

        Self::get_by_id(pool, id)
            .await?
            .ok_or_else(|| DbError::NotFound {
                entity: "watched_directory",
                id: id.to_string(),
            })
    }

    pub async fn set_last_error(
        pool: &SqlitePool,
        id: Uuid,
        error: Option<&str>,
    ) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query!(
            "UPDATE watched_directories SET last_error = ? WHERE id = ?",
            error,
            id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "watched_directory",
                id: id_str,
            });
        }

        Ok(())
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query!("DELETE FROM watched_directories WHERE id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "watched_directory",
                id: id_str,
            });
        }

        Ok(())
    }

    pub async fn exists_by_path(pool: &SqlitePool, path: &str) -> Result<bool, DbError> {
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM watched_directories WHERE path = ?",
            path,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(count > 0)
    }
}
