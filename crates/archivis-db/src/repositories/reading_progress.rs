use archivis_core::errors::DbError;
use archivis_core::models::ReadingProgress;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct ReadingProgressRepository;

impl ReadingProgressRepository {
    /// Insert or update a reading progress record.
    ///
    /// The UNIQUE constraint is on `(user_id, book_file_id, device_id)`.
    /// `SQLite` treats NULL `device_id` as distinct for uniqueness, so we handle
    /// the conflict detection manually for NULL `device_id` cases.
    pub async fn upsert(
        pool: &SqlitePool,
        user_id: Uuid,
        book_file_id: Uuid,
        book_id: Uuid,
        location: Option<&str>,
        progress: f64,
        device_id: Option<&str>,
        preferences: Option<&serde_json::Value>,
    ) -> Result<ReadingProgress, DbError> {
        let id = Uuid::new_v4().to_string();
        let user_id_str = user_id.to_string();
        let book_file_id_str = book_file_id.to_string();
        let book_id_str = book_id.to_string();
        let preferences_str = preferences.map(std::string::ToString::to_string);
        let now = Utc::now().to_rfc3339();

        // For NULL device_id, SQLite's UNIQUE constraint treats each NULL as distinct,
        // so ON CONFLICT won't fire. We do a manual check-and-update instead.
        if device_id.is_none() {
            let existing = sqlx::query_as!(
                ReadingProgressRow,
                r#"SELECT id, user_id, book_id, book_file_id, location, progress,
                          device_id, preferences, started_at, updated_at
                   FROM reading_progress
                   WHERE user_id = ? AND book_file_id = ? AND device_id IS NULL"#,
                user_id_str,
                book_file_id_str,
            )
            .fetch_optional(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

            if let Some(row) = existing {
                sqlx::query!(
                    r#"UPDATE reading_progress
                       SET location = ?, progress = ?, preferences = ?, updated_at = ?
                       WHERE id = ?"#,
                    location,
                    progress,
                    preferences_str,
                    now,
                    row.id,
                )
                .execute(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;

                // Re-fetch the updated row
                return Self::get_by_id(pool, &row.id).await;
            }
        }

        sqlx::query!(
            r#"INSERT INTO reading_progress (id, user_id, book_id, book_file_id, location, progress, device_id, preferences, started_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT (user_id, book_file_id, device_id)
               DO UPDATE SET location = excluded.location, progress = excluded.progress,
                             preferences = excluded.preferences, updated_at = excluded.updated_at"#,
            id,
            user_id_str,
            book_id_str,
            book_file_id_str,
            location,
            progress,
            device_id,
            preferences_str,
            now,
            now,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        // For non-NULL device_id, the ON CONFLICT may have updated an existing row,
        // so we need to fetch the actual row (which may have a different id).
        if device_id.is_some() {
            let row = sqlx::query_as!(
                ReadingProgressRow,
                r#"SELECT id, user_id, book_id, book_file_id, location, progress,
                          device_id, preferences, started_at, updated_at
                   FROM reading_progress
                   WHERE user_id = ? AND book_file_id = ? AND device_id = ?"#,
                user_id_str,
                book_file_id_str,
                device_id,
            )
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

            return row.into_reading_progress();
        }

        Self::get_by_id(pool, &id).await
    }

    /// Get the most recently updated reading progress for a book (any file).
    pub async fn get_for_book(
        pool: &SqlitePool,
        user_id: Uuid,
        book_id: Uuid,
    ) -> Result<Option<ReadingProgress>, DbError> {
        let user_id_str = user_id.to_string();
        let book_id_str = book_id.to_string();

        let row = sqlx::query_as!(
            ReadingProgressRow,
            r#"SELECT id, user_id, book_id, book_file_id, location, progress,
                      device_id, preferences, started_at, updated_at
               FROM reading_progress
               WHERE user_id = ? AND book_id = ?
               ORDER BY updated_at DESC
               LIMIT 1"#,
            user_id_str,
            book_id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        row.map(ReadingProgressRow::into_reading_progress)
            .transpose()
    }

    /// Get reading progress for a specific file and device.
    ///
    /// Handles NULL `device_id` comparison correctly: if `device_id` is `None`,
    /// matches rows where `device_id IS NULL`.
    pub async fn get_for_file(
        pool: &SqlitePool,
        user_id: Uuid,
        book_file_id: Uuid,
        device_id: Option<&str>,
    ) -> Result<Option<ReadingProgress>, DbError> {
        let user_id_str = user_id.to_string();
        let book_file_id_str = book_file_id.to_string();

        let row = sqlx::query_as!(
            ReadingProgressRow,
            r#"SELECT id, user_id, book_id, book_file_id, location, progress,
                      device_id, preferences, started_at, updated_at
               FROM reading_progress
               WHERE user_id = ? AND book_file_id = ?
                 AND (device_id = ? OR (device_id IS NULL AND ? IS NULL))"#,
            user_id_str,
            book_file_id_str,
            device_id,
            device_id,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        row.map(ReadingProgressRow::into_reading_progress)
            .transpose()
    }

    /// List the most recently updated reading progress records for a user.
    pub async fn list_recent(
        pool: &SqlitePool,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ReadingProgress>, DbError> {
        let user_id_str = user_id.to_string();

        let rows = sqlx::query_as!(
            ReadingProgressRow,
            r#"SELECT id, user_id, book_id, book_file_id, location, progress,
                      device_id, preferences, started_at, updated_at
               FROM reading_progress
               WHERE user_id = ?
               ORDER BY updated_at DESC
               LIMIT ?"#,
            user_id_str,
            limit,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(ReadingProgressRow::into_reading_progress)
            .collect()
    }

    /// Delete all reading progress records for a user+book combination.
    pub async fn delete_for_book(
        pool: &SqlitePool,
        user_id: Uuid,
        book_id: Uuid,
    ) -> Result<u64, DbError> {
        let user_id_str = user_id.to_string();
        let book_id_str = book_id.to_string();

        let result = sqlx::query!(
            "DELETE FROM reading_progress WHERE user_id = ? AND book_id = ?",
            user_id_str,
            book_id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(result.rows_affected())
    }

    async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<ReadingProgress, DbError> {
        let row = sqlx::query_as!(
            ReadingProgressRow,
            r#"SELECT id, user_id, book_id, book_file_id, location, progress,
                      device_id, preferences, started_at, updated_at
               FROM reading_progress
               WHERE id = ?"#,
            id,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or_else(|| DbError::NotFound {
            entity: "reading_progress",
            id: id.to_string(),
        })?;

        row.into_reading_progress()
    }
}

// ── Row type for sqlx mapping ──────────────────────────────────

#[derive(sqlx::FromRow)]
struct ReadingProgressRow {
    id: String,
    user_id: String,
    book_id: String,
    book_file_id: String,
    location: Option<String>,
    progress: f64,
    device_id: Option<String>,
    preferences: Option<String>,
    started_at: String,
    updated_at: String,
}

impl ReadingProgressRow {
    fn into_reading_progress(self) -> Result<ReadingProgress, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid reading_progress UUID: {e}")))?;
        let user_id = Uuid::parse_str(&self.user_id)
            .map_err(|e| DbError::Query(format!("invalid user UUID: {e}")))?;
        let book_id = Uuid::parse_str(&self.book_id)
            .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
        let book_file_id = Uuid::parse_str(&self.book_file_id)
            .map_err(|e| DbError::Query(format!("invalid book_file UUID: {e}")))?;
        let started_at = parse_datetime(&self.started_at, "started_at")?;
        let updated_at = parse_datetime(&self.updated_at, "updated_at")?;
        let preferences = self
            .preferences
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| DbError::Query(format!("invalid preferences JSON: {e}")))?;

        Ok(ReadingProgress {
            id,
            user_id,
            book_id,
            book_file_id,
            location: self.location,
            progress: self.progress,
            device_id: self.device_id,
            preferences,
            started_at,
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
