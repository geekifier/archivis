use archivis_core::errors::DbError;
use archivis_core::models::{User, UserRole};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct UserRepository;

impl UserRepository {
    pub async fn create(pool: &SqlitePool, user: &User) -> Result<(), DbError> {
        let id = user.id.to_string();
        let role = user.role.to_string();
        let created_at = user.created_at.to_rfc3339();
        let is_active = i64::from(user.is_active);

        sqlx::query(
            "INSERT INTO users (id, username, email, password_hash, role, created_at, is_active) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&user.username)
        .bind(&user.email)
        .bind(&user.password_hash)
        .bind(&role)
        .bind(&created_at)
        .bind(is_active)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<User, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, username, email, password_hash, role, created_at, is_active FROM users WHERE id = ?",
        )
        .bind(&id_str)
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "user",
            id: id_str,
        })?;

        row.into_user()
    }

    pub async fn get_by_username(pool: &SqlitePool, username: &str) -> Result<User, DbError> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, username, email, password_hash, role, created_at, is_active FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or_else(|| DbError::NotFound {
            entity: "user",
            id: username.to_string(),
        })?;

        row.into_user()
    }

    pub async fn update(pool: &SqlitePool, user: &User) -> Result<(), DbError> {
        let id = user.id.to_string();
        let role = user.role.to_string();
        let is_active = i64::from(user.is_active);

        let result = sqlx::query(
            "UPDATE users SET username = ?, email = ?, role = ?, is_active = ? WHERE id = ?",
        )
        .bind(&user.username)
        .bind(&user.email)
        .bind(&role)
        .bind(is_active)
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound { entity: "user", id });
        }

        Ok(())
    }

    pub async fn update_password(
        pool: &SqlitePool,
        id: Uuid,
        password_hash: &str,
    ) -> Result<(), DbError> {
        let id_str = id.to_string();
        let result = sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
            .bind(password_hash)
            .bind(&id_str)
            .execute(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "user",
                id: id_str,
            });
        }

        Ok(())
    }

    pub async fn list(pool: &SqlitePool) -> Result<Vec<User>, DbError> {
        let rows = sqlx::query_as::<_, UserRow>(
            "SELECT id, username, email, password_hash, role, created_at, is_active FROM users ORDER BY username ASC",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(UserRow::into_user).collect()
    }

    pub async fn count(pool: &SqlitePool) -> Result<i64, DbError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(count)
    }
}

// ── Row type for sqlx mapping ──────────────────────────────────

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    username: String,
    email: Option<String>,
    password_hash: String,
    role: String,
    created_at: String,
    is_active: i64,
}

impl UserRow {
    fn into_user(self) -> Result<User, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid user UUID: {e}")))?;
        let role: UserRole = self.role.parse().map_err(|e: String| DbError::Query(e))?;
        let created_at = parse_datetime(&self.created_at, "created_at")?;

        Ok(User {
            id,
            username: self.username,
            email: self.email,
            password_hash: self.password_hash,
            role,
            created_at,
            is_active: self.is_active != 0,
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
