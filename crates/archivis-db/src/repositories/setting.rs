use archivis_core::errors::DbError;
use sqlx::SqlitePool;

pub struct SettingRepository;

impl SettingRepository {
    pub async fn get_all(pool: &SqlitePool) -> Result<Vec<(String, String)>, DbError> {
        let rows = sqlx::query!("SELECT key, value FROM settings ORDER BY key")
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(rows.into_iter().map(|r| (r.key, r.value)).collect())
    }

    pub async fn get(pool: &SqlitePool, key: &str) -> Result<Option<String>, DbError> {
        let row = sqlx::query!("SELECT value FROM settings WHERE key = ?", key)
            .fetch_optional(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(row.map(|r| r.value))
    }

    pub async fn set(pool: &SqlitePool, key: &str, value: &str) -> Result<(), DbError> {
        sqlx::query!(
            "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = datetime('now')",
            key,
            value,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(())
    }

    pub async fn delete(pool: &SqlitePool, key: &str) -> Result<(), DbError> {
        sqlx::query!("DELETE FROM settings WHERE key = ?", key)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;
        Ok(())
    }

    pub async fn set_many(pool: &SqlitePool, entries: &[(String, String)]) -> Result<(), DbError> {
        for (key, value) in entries {
            Self::set(pool, key, value).await?;
        }
        Ok(())
    }
}
