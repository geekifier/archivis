use archivis_core::errors::DbError;
use archivis_core::models::{CandidateStatus, IdentificationCandidate};
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct CandidateRepository;

impl CandidateRepository {
    /// Insert a new identification candidate.
    pub async fn create(
        pool: &SqlitePool,
        candidate: &IdentificationCandidate,
    ) -> Result<(), DbError> {
        let id = candidate.id.to_string();
        let book_id = candidate.book_id.to_string();
        let metadata_json = serde_json::to_string(&candidate.metadata)
            .map_err(|e| DbError::Query(format!("failed to serialize metadata: {e}")))?;
        let reasons_json = serde_json::to_string(&candidate.match_reasons)
            .map_err(|e| DbError::Query(format!("failed to serialize match_reasons: {e}")))?;
        let status = candidate.status.to_string();
        let created_at = candidate.created_at.to_rfc3339();

        sqlx::query!(
            "INSERT INTO identification_candidates (id, book_id, provider_name, score, metadata, match_reasons, status, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            id,
            book_id,
            candidate.provider_name,
            candidate.score,
            metadata_json,
            reasons_json,
            status,
            created_at,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// List all candidates for a book, sorted by score descending.
    pub async fn list_by_book(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Vec<IdentificationCandidate>, DbError> {
        let book_id_str = book_id.to_string();
        let rows = sqlx::query_as!(
            CandidateRow,
            "SELECT id, book_id, provider_name, score, metadata, match_reasons, status, created_at FROM identification_candidates WHERE book_id = ? ORDER BY score DESC",
            book_id_str,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(CandidateRow::into_candidate).collect()
    }

    /// Get a single candidate by ID.
    pub async fn get_by_id(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<IdentificationCandidate>, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            CandidateRow,
            "SELECT id, book_id, provider_name, score, metadata, match_reasons, status, created_at FROM identification_candidates WHERE id = ?",
            id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        row.map(CandidateRow::into_candidate).transpose()
    }

    /// Update the status of a candidate.
    pub async fn update_status(
        pool: &SqlitePool,
        id: Uuid,
        status: CandidateStatus,
    ) -> Result<(), DbError> {
        let id_str = id.to_string();
        let status_str = status.to_string();

        let result = sqlx::query!(
            "UPDATE identification_candidates SET status = ? WHERE id = ?",
            status_str,
            id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "identification_candidate",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Delete all candidates for a book.
    pub async fn delete_by_book(pool: &SqlitePool, book_id: Uuid) -> Result<(), DbError> {
        let book_id_str = book_id.to_string();
        sqlx::query!(
            "DELETE FROM identification_candidates WHERE book_id = ?",
            book_id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }
}

// ── Row type for sqlx mapping ──────────────────────────────────

#[derive(sqlx::FromRow)]
struct CandidateRow {
    id: String,
    book_id: String,
    provider_name: String,
    score: f64,
    metadata: String,
    match_reasons: Option<String>,
    status: String,
    created_at: String,
}

impl CandidateRow {
    fn into_candidate(self) -> Result<IdentificationCandidate, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid candidate UUID: {e}")))?;
        let book_id = Uuid::parse_str(&self.book_id)
            .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
        let metadata: serde_json::Value = serde_json::from_str(&self.metadata)
            .map_err(|e| DbError::Query(format!("invalid metadata JSON: {e}")))?;
        let match_reasons: Vec<String> = self
            .match_reasons
            .as_deref()
            .map(|s| serde_json::from_str(s).unwrap_or_default())
            .unwrap_or_default();
        let status: CandidateStatus = self.status.parse().map_err(|e: String| DbError::Query(e))?;
        let created_at = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map(|d| d.with_timezone(&chrono::Utc))
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(&self.created_at, "%Y-%m-%dT%H:%M:%S%.fZ")
                    .map(|ndt| ndt.and_utc())
            })
            .map_err(|e| DbError::Query(format!("invalid created_at: {e}")))?;

        #[allow(clippy::cast_possible_truncation)]
        Ok(IdentificationCandidate {
            id,
            book_id,
            provider_name: self.provider_name,
            score: self.score as f32,
            metadata,
            match_reasons,
            status,
            created_at,
        })
    }
}
