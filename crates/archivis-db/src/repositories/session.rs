use archivis_core::errors::DbError;
use archivis_core::models::Session;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct SessionRepository;

impl SessionRepository {
    pub async fn create(pool: &SqlitePool, session: &Session) -> Result<(), DbError> {
        let id = session.id.to_string();
        let user_id = session.user_id.to_string();
        let expires_at = session.expires_at.to_rfc3339();
        let created_at = session.created_at.to_rfc3339();

        sqlx::query!(
            "INSERT INTO sessions (id, user_id, token_hash, expires_at, created_at) VALUES (?, ?, ?, ?, ?)",
            id,
            user_id,
            session.token_hash,
            expires_at,
            created_at,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn get_by_token_hash(
        pool: &SqlitePool,
        token_hash: &str,
    ) -> Result<Session, DbError> {
        let row = sqlx::query_as!(
            SessionRow,
            "SELECT id, user_id, token_hash, expires_at, created_at FROM sessions WHERE token_hash = ?",
            token_hash,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or_else(|| DbError::NotFound {
            entity: "session",
            id: "by_token_hash".to_string(),
        })?;

        row.into_session()
    }

    pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query!("DELETE FROM sessions WHERE id = ?", id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "session",
                id: id_str,
            });
        }

        Ok(())
    }

    pub async fn delete_by_user(pool: &SqlitePool, user_id: Uuid) -> Result<(), DbError> {
        let user_id_str = user_id.to_string();
        sqlx::query!("DELETE FROM sessions WHERE user_id = ?", user_id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn delete_expired(pool: &SqlitePool) -> Result<u64, DbError> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query!("DELETE FROM sessions WHERE expires_at < ?", now)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(result.rows_affected())
    }
}

// ── Row type for sqlx mapping ──────────────────────────────────

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    user_id: String,
    token_hash: String,
    expires_at: String,
    created_at: String,
}

impl SessionRow {
    fn into_session(self) -> Result<Session, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid session UUID: {e}")))?;
        let user_id = Uuid::parse_str(&self.user_id)
            .map_err(|e| DbError::Query(format!("invalid session user_id UUID: {e}")))?;
        let expires_at = parse_datetime(&self.expires_at, "expires_at")?;
        let created_at = parse_datetime(&self.created_at, "created_at")?;

        Ok(Session {
            id,
            user_id,
            token_hash: self.token_hash,
            expires_at,
            created_at,
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
