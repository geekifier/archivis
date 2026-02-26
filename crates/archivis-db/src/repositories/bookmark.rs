use archivis_core::errors::DbError;
use archivis_core::models::Bookmark;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct BookmarkRepository;

impl BookmarkRepository {
    /// Create a new bookmark.
    pub async fn create(pool: &SqlitePool, bookmark: &Bookmark) -> Result<(), DbError> {
        let id = bookmark.id.to_string();
        let user_id = bookmark.user_id.to_string();
        let book_id = bookmark.book_id.to_string();
        let book_file_id = bookmark.book_file_id.to_string();
        let created_at = bookmark.created_at.to_rfc3339();

        sqlx::query!(
            r#"INSERT INTO bookmarks (id, user_id, book_id, book_file_id, location, label, excerpt, position, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            id,
            user_id,
            book_id,
            book_file_id,
            bookmark.location,
            bookmark.label,
            bookmark.excerpt,
            bookmark.position,
            created_at,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// List all bookmarks for a user+file, ordered by position.
    pub async fn list_for_file(
        pool: &SqlitePool,
        user_id: Uuid,
        book_file_id: Uuid,
    ) -> Result<Vec<Bookmark>, DbError> {
        let user_id_str = user_id.to_string();
        let book_file_id_str = book_file_id.to_string();

        let rows = sqlx::query_as!(
            BookmarkRow,
            r#"SELECT id, user_id, book_id, book_file_id, location, label, excerpt, position, created_at
               FROM bookmarks
               WHERE user_id = ? AND book_file_id = ?
               ORDER BY position ASC"#,
            user_id_str,
            book_file_id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(BookmarkRow::into_bookmark).collect()
    }

    /// Delete a bookmark with ownership check.
    /// Returns `DbError::NotFound` if no row matches (wrong id or wrong user).
    pub async fn delete(pool: &SqlitePool, id: Uuid, user_id: Uuid) -> Result<(), DbError> {
        let id_str = id.to_string();
        let user_id_str = user_id.to_string();

        let result = sqlx::query!(
            "DELETE FROM bookmarks WHERE id = ? AND user_id = ?",
            id_str,
            user_id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "bookmark",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Update a bookmark's label with ownership check.
    pub async fn update_label(
        pool: &SqlitePool,
        id: Uuid,
        user_id: Uuid,
        label: Option<&str>,
    ) -> Result<(), DbError> {
        let id_str = id.to_string();
        let user_id_str = user_id.to_string();

        let result = sqlx::query!(
            "UPDATE bookmarks SET label = ? WHERE id = ? AND user_id = ?",
            label,
            id_str,
            user_id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "bookmark",
                id: id_str,
            });
        }

        Ok(())
    }
}

// ── Row type for sqlx mapping ──────────────────────────────────

#[derive(sqlx::FromRow)]
struct BookmarkRow {
    id: String,
    user_id: String,
    book_id: String,
    book_file_id: String,
    location: String,
    label: Option<String>,
    excerpt: Option<String>,
    position: f64,
    created_at: String,
}

impl BookmarkRow {
    fn into_bookmark(self) -> Result<Bookmark, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid bookmark UUID: {e}")))?;
        let user_id = Uuid::parse_str(&self.user_id)
            .map_err(|e| DbError::Query(format!("invalid user UUID: {e}")))?;
        let book_id = Uuid::parse_str(&self.book_id)
            .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
        let book_file_id = Uuid::parse_str(&self.book_file_id)
            .map_err(|e| DbError::Query(format!("invalid book_file UUID: {e}")))?;
        let created_at = parse_datetime(&self.created_at, "created_at")?;

        Ok(Bookmark {
            id,
            user_id,
            book_id,
            book_file_id,
            location: self.location,
            label: self.label,
            excerpt: self.excerpt,
            position: self.position,
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
