use archivis_core::errors::DbError;
use archivis_core::models::{DuplicateLink, DuplicateStatus};
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct DuplicateRepository;

impl DuplicateRepository {
    /// Insert a new duplicate link.
    pub async fn create(pool: &SqlitePool, link: &DuplicateLink) -> Result<(), DbError> {
        let id = link.id.to_string();
        let book_id_a = link.book_id_a.to_string();
        let book_id_b = link.book_id_b.to_string();
        let status = link.status.to_string();
        let created_at = link.created_at.to_rfc3339();

        sqlx::query!(
            "INSERT INTO duplicate_links (id, book_id_a, book_id_b, detection_method, confidence, status, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            id,
            book_id_a,
            book_id_b,
            link.detection_method,
            link.confidence,
            status,
            created_at,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// List pending duplicate links with pagination.
    pub async fn list_pending(
        pool: &SqlitePool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DuplicateLink>, DbError> {
        let rows = sqlx::query_as!(
            DuplicateLinkRow,
            "SELECT id, book_id_a, book_id_b, detection_method, confidence, status, created_at FROM duplicate_links WHERE status = 'pending' ORDER BY created_at DESC LIMIT ? OFFSET ?",
            limit,
            offset,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(DuplicateLinkRow::into_duplicate_link)
            .collect()
    }

    /// Count pending duplicate links.
    pub async fn count_pending(pool: &SqlitePool) -> Result<i64, DbError> {
        let count =
            sqlx::query_scalar!("SELECT COUNT(*) FROM duplicate_links WHERE status = 'pending'")
                .fetch_one(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(count)
    }

    /// Get a single duplicate link by ID.
    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<DuplicateLink, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            DuplicateLinkRow,
            "SELECT id, book_id_a, book_id_b, detection_method, confidence, status, created_at FROM duplicate_links WHERE id = ?",
            id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "duplicate_link",
            id: id_str,
        })?;

        row.into_duplicate_link()
    }

    /// Find all duplicate links involving a specific book (either side).
    pub async fn find_for_book(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Vec<DuplicateLink>, DbError> {
        let book_id_str = book_id.to_string();
        let rows = sqlx::query_as!(
            DuplicateLinkRow,
            "SELECT id, book_id_a, book_id_b, detection_method, confidence, status, created_at FROM duplicate_links WHERE book_id_a = ? OR book_id_b = ? ORDER BY created_at DESC",
            book_id_str,
            book_id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter()
            .map(DuplicateLinkRow::into_duplicate_link)
            .collect()
    }

    /// Update the status of a duplicate link.
    pub async fn update_status(pool: &SqlitePool, id: Uuid, status: &str) -> Result<(), DbError> {
        let id_str = id.to_string();

        let result = sqlx::query!(
            "UPDATE duplicate_links SET status = ? WHERE id = ?",
            status,
            id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "duplicate_link",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Check if a duplicate link already exists between two books (in either direction).
    pub async fn exists(
        pool: &SqlitePool,
        book_id_a: Uuid,
        book_id_b: Uuid,
    ) -> Result<bool, DbError> {
        let a_str = book_id_a.to_string();
        let b_str = book_id_b.to_string();

        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM duplicate_links WHERE (book_id_a = ? AND book_id_b = ?) OR (book_id_a = ? AND book_id_b = ?)",
            a_str,
            b_str,
            b_str,
            a_str,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(count > 0)
    }
}

// ── Row type for sqlx mapping ──────────────────────────────────

#[derive(sqlx::FromRow)]
struct DuplicateLinkRow {
    id: String,
    book_id_a: String,
    book_id_b: String,
    detection_method: String,
    confidence: f64,
    status: String,
    created_at: String,
}

impl DuplicateLinkRow {
    fn into_duplicate_link(self) -> Result<DuplicateLink, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid duplicate_link UUID: {e}")))?;
        let book_id_a = Uuid::parse_str(&self.book_id_a)
            .map_err(|e| DbError::Query(format!("invalid book UUID (a): {e}")))?;
        let book_id_b = Uuid::parse_str(&self.book_id_b)
            .map_err(|e| DbError::Query(format!("invalid book UUID (b): {e}")))?;
        let status: DuplicateStatus = self.status.parse().map_err(|e: String| DbError::Query(e))?;
        let created_at = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map(|d| d.with_timezone(&chrono::Utc))
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(&self.created_at, "%Y-%m-%dT%H:%M:%S%.fZ")
                    .map(|ndt| ndt.and_utc())
            })
            .map_err(|e| DbError::Query(format!("invalid created_at: {e}")))?;

        #[allow(clippy::cast_possible_truncation)]
        Ok(DuplicateLink {
            id,
            book_id_a,
            book_id_b,
            detection_method: self.detection_method,
            confidence: self.confidence as f32,
            status,
            created_at,
        })
    }
}
