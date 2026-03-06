use std::collections::{HashMap, HashSet};

use archivis_core::errors::DbError;
use archivis_core::models::{
    CandidateStatus, IdentificationCandidate, ResolutionOutcome, ResolutionRun, ResolutionRunState,
};
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

use crate::ResolutionRunRepository;

pub struct CandidateRepository;

const REVIEW_CANDIDATE_RETENTION_LIMIT: usize = 5;
const SUPERSEDED_RUN_RETENTION_LIMIT: usize = 2;

impl CandidateRepository {
    /// Insert a new identification candidate.
    pub async fn create(
        pool: &SqlitePool,
        candidate: &IdentificationCandidate,
    ) -> Result<(), DbError> {
        let run_id = candidate.run_id.as_ref().map(ToString::to_string);
        let metadata_json = serde_json::to_string(&candidate.metadata)
            .map_err(|e| DbError::Query(format!("failed to serialize metadata: {e}")))?;
        let reasons_json = serde_json::to_string(&candidate.match_reasons)
            .map_err(|e| DbError::Query(format!("failed to serialize match_reasons: {e}")))?;
        let disputes_json = serde_json::to_string(&candidate.disputes)
            .map_err(|e| DbError::Query(format!("failed to serialize disputes: {e}")))?;

        sqlx::query(
            "INSERT INTO identification_candidates (
                 id, book_id, run_id, provider_name, score, metadata, match_reasons,
                 disputes, status, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(candidate.id.to_string())
        .bind(candidate.book_id.to_string())
        .bind(run_id)
        .bind(&candidate.provider_name)
        .bind(candidate.score)
        .bind(metadata_json)
        .bind(reasons_json)
        .bind(disputes_json)
        .bind(candidate.status.to_string())
        .bind(candidate.created_at.to_rfc3339())
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    /// List the current reviewable candidates for a book.
    ///
    /// Compatibility shim: older callers still use `list_by_book`, but it now
    /// returns only the latest reviewable run (or legacy no-run candidates) so
    /// historical rows are preserved.
    pub async fn list_by_book(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Vec<IdentificationCandidate>, DbError> {
        Self::list_current_reviewable_by_book(pool, book_id).await
    }

    /// List all historical candidates for a book across all runs.
    pub async fn list_all_by_book(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Vec<IdentificationCandidate>, DbError> {
        let rows = sqlx::query_as::<_, CandidateRow>(
            "SELECT id, book_id, run_id, provider_name, score, metadata, match_reasons,
                    disputes, status, created_at
             FROM identification_candidates
             WHERE book_id = ?
             ORDER BY created_at DESC, score DESC",
        )
        .bind(book_id.to_string())
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(CandidateRow::into_candidate).collect()
    }

    /// List all candidates attached to a specific resolution run.
    pub async fn list_by_run(
        pool: &SqlitePool,
        run_id: Uuid,
    ) -> Result<Vec<IdentificationCandidate>, DbError> {
        let rows = sqlx::query_as::<_, CandidateRow>(
            "SELECT id, book_id, run_id, provider_name, score, metadata, match_reasons,
                    disputes, status, created_at
             FROM identification_candidates
             WHERE run_id = ?
             ORDER BY score DESC, created_at ASC",
        )
        .bind(run_id.to_string())
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(CandidateRow::into_candidate).collect()
    }

    /// List the latest reviewable run's candidates for a book.
    pub async fn list_current_reviewable_by_book(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Vec<IdentificationCandidate>, DbError> {
        if let Some(run_id) = Self::latest_reviewable_run_id(pool, book_id).await? {
            return Self::list_current_reviewable_by_run(pool, run_id).await;
        }

        let rows = sqlx::query_as::<_, CandidateRow>(
            "SELECT id, book_id, run_id, provider_name, score, metadata, match_reasons,
                    disputes, status, created_at
             FROM identification_candidates
             WHERE book_id = ?
               AND run_id IS NULL
               AND status != 'superseded'
             ORDER BY score DESC, created_at ASC",
        )
        .bind(book_id.to_string())
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(CandidateRow::into_candidate).collect()
    }

    /// List legacy runless candidates for a book.
    pub async fn list_legacy_by_book(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Vec<IdentificationCandidate>, DbError> {
        let rows = sqlx::query_as::<_, CandidateRow>(
            "SELECT id, book_id, run_id, provider_name, score, metadata, match_reasons,
                    disputes, status, created_at
             FROM identification_candidates
             WHERE book_id = ?
               AND run_id IS NULL
             ORDER BY score DESC, created_at ASC",
        )
        .bind(book_id.to_string())
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
        let row = sqlx::query_as::<_, CandidateRow>(
            "SELECT id, book_id, run_id, provider_name, score, metadata, match_reasons,
                    disputes, status, created_at
             FROM identification_candidates
             WHERE id = ?",
        )
        .bind(id.to_string())
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

        let result = sqlx::query(
            "UPDATE identification_candidates
             SET status = ?
             WHERE id = ?",
        )
        .bind(status_str)
        .bind(id_str.clone())
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

    pub async fn update_status_conn(
        conn: &mut SqliteConnection,
        id: Uuid,
        status: CandidateStatus,
    ) -> Result<(), DbError> {
        let id_str = id.to_string();
        let status_str = status.to_string();

        let result = sqlx::query(
            "UPDATE identification_candidates
             SET status = ?
             WHERE id = ?",
        )
        .bind(status_str)
        .bind(id_str.clone())
        .execute(conn)
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

    /// Update the disputes JSON on a candidate.
    pub async fn update_disputes(
        pool: &SqlitePool,
        id: Uuid,
        disputes: &[String],
    ) -> Result<(), DbError> {
        let disputes_json = serde_json::to_string(disputes)
            .map_err(|e| DbError::Query(format!("failed to serialize disputes: {e}")))?;
        let id_str = id.to_string();

        let result = sqlx::query(
            "UPDATE identification_candidates
             SET disputes = ?
             WHERE id = ?",
        )
        .bind(disputes_json)
        .bind(id_str.clone())
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

    /// Mark every candidate in a run as superseded.
    pub async fn mark_run_superseded(pool: &SqlitePool, run_id: Uuid) -> Result<u64, DbError> {
        let superseded = CandidateStatus::Superseded.to_string();
        let result = sqlx::query(
            "UPDATE identification_candidates
             SET status = ?
             WHERE run_id = ?
               AND status != ?",
        )
        .bind(&superseded)
        .bind(run_id.to_string())
        .bind(&superseded)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(result.rows_affected())
    }

    /// Mark all candidates for other runs on the same book as superseded.
    pub async fn mark_other_runs_superseded(
        pool: &SqlitePool,
        book_id: Uuid,
        keep_run_id: Uuid,
    ) -> Result<u64, DbError> {
        let superseded = CandidateStatus::Superseded.to_string();
        let mut affected = 0;

        let result = sqlx::query(
            "UPDATE identification_candidates
             SET status = ?
             WHERE book_id = ?
               AND status != ?
               AND (run_id IS NULL OR run_id != ?)",
        )
        .bind(&superseded)
        .bind(book_id.to_string())
        .bind(&superseded)
        .bind(keep_run_id.to_string())
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
        affected += result.rows_affected();

        Ok(affected)
    }

    /// Compatibility shim for older destructive pruning call sites.
    pub async fn delete_by_book(pool: &SqlitePool, book_id: Uuid) -> Result<(), DbError> {
        if let Some(run_id) = Self::latest_reviewable_run_id(pool, book_id).await? {
            Self::mark_run_superseded(pool, run_id).await?;
        } else {
            Self::supersede_legacy_candidates(pool, book_id).await?;
        }

        Ok(())
    }

    async fn latest_reviewable_run_id(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Option<Uuid>, DbError> {
        let run_id = sqlx::query_scalar::<_, String>(
            "SELECT rr.id
             FROM resolution_runs rr
             WHERE rr.book_id = ?
               AND rr.state != 'superseded'
               AND (
                   rr.outcome IN ('disputed', 'ambiguous')
                   OR EXISTS (
                       SELECT 1
                       FROM identification_candidates c
                       WHERE c.run_id = rr.id
                         AND c.status = 'pending'
                   )
               )
             ORDER BY rr.started_at DESC
             LIMIT 1",
        )
        .bind(book_id.to_string())
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        run_id
            .map(|value| {
                Uuid::parse_str(&value)
                    .map_err(|e| DbError::Query(format!("invalid resolution run UUID: {e}")))
            })
            .transpose()
    }

    async fn list_current_reviewable_by_run(
        pool: &SqlitePool,
        run_id: Uuid,
    ) -> Result<Vec<IdentificationCandidate>, DbError> {
        let rows = sqlx::query_as::<_, CandidateRow>(
            "SELECT id, book_id, run_id, provider_name, score, metadata, match_reasons,
                    disputes, status, created_at
             FROM identification_candidates
             WHERE run_id = ?
               AND status != 'superseded'
             ORDER BY score DESC, created_at ASC",
        )
        .bind(run_id.to_string())
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(CandidateRow::into_candidate).collect()
    }

    async fn supersede_legacy_candidates(pool: &SqlitePool, book_id: Uuid) -> Result<u64, DbError> {
        let superseded = CandidateStatus::Superseded.to_string();
        let result = sqlx::query(
            "UPDATE identification_candidates
             SET status = ?
             WHERE book_id = ?
               AND run_id IS NULL
               AND status != ?",
        )
        .bind(&superseded)
        .bind(book_id.to_string())
        .bind(&superseded)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(result.rows_affected())
    }

    /// Prune historical candidates down to the rows still useful for review.
    ///
    /// Keeps:
    /// - the applied candidate for any current book state
    /// - up to the top five candidates for active ambiguous/disputed runs
    /// - up to the top five candidates for the last two superseded runs
    /// - up to the top five legacy runless candidates when a book has no run history yet
    ///
    /// Everything else is deleted to keep candidate storage bounded.
    pub async fn prune_by_usefulness(pool: &SqlitePool, book_id: Uuid) -> Result<u64, DbError> {
        let runs = ResolutionRunRepository::list_by_book(pool, book_id).await?;
        let candidates = Self::list_all_by_book(pool, book_id).await?;

        let active_review_run_ids: HashSet<Uuid> = runs
            .iter()
            .filter(|run| is_reviewable_run(run) && run.state != ResolutionRunState::Superseded)
            .map(|run| run.id)
            .collect();
        let retained_superseded_run_ids: HashSet<Uuid> = runs
            .iter()
            .filter(|run| run.state == ResolutionRunState::Superseded)
            .take(SUPERSEDED_RUN_RETENTION_LIMIT)
            .map(|run| run.id)
            .collect();

        let mut top_candidate_ids = top_candidate_ids_for_runs(
            &candidates,
            &active_review_run_ids,
            REVIEW_CANDIDATE_RETENTION_LIMIT,
        );
        top_candidate_ids.extend(top_candidate_ids_for_runs(
            &candidates,
            &retained_superseded_run_ids,
            REVIEW_CANDIDATE_RETENTION_LIMIT,
        ));

        let legacy_candidate_ids = if runs.is_empty() {
            top_candidate_ids_for_legacy_candidates(&candidates, REVIEW_CANDIDATE_RETENTION_LIMIT)
        } else {
            HashSet::new()
        };

        let delete_ids: Vec<Uuid> = candidates
            .iter()
            .filter(|candidate| {
                !should_keep_candidate(candidate, &top_candidate_ids, &legacy_candidate_ids)
            })
            .map(|candidate| candidate.id)
            .collect();

        let mut deleted = 0;
        for candidate_id in delete_ids {
            deleted += sqlx::query("DELETE FROM identification_candidates WHERE id = ?")
                .bind(candidate_id.to_string())
                .execute(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?
                .rows_affected();
        }

        Ok(deleted)
    }
}

fn is_reviewable_run(run: &ResolutionRun) -> bool {
    matches!(
        run.outcome,
        Some(ResolutionOutcome::Ambiguous | ResolutionOutcome::Disputed)
    )
}

fn top_candidate_ids_for_runs(
    candidates: &[IdentificationCandidate],
    run_ids: &HashSet<Uuid>,
    limit: usize,
) -> HashSet<Uuid> {
    let mut grouped: HashMap<Uuid, Vec<&IdentificationCandidate>> = HashMap::new();

    for candidate in candidates.iter().filter(|candidate| {
        candidate
            .run_id
            .is_some_and(|run_id| run_ids.contains(&run_id))
    }) {
        grouped
            .entry(candidate.run_id.expect("filtered above"))
            .or_default()
            .push(candidate);
    }

    grouped
        .into_values()
        .flat_map(|entries| top_candidate_ids(entries, limit))
        .collect()
}

fn top_candidate_ids_for_legacy_candidates(
    candidates: &[IdentificationCandidate],
    limit: usize,
) -> HashSet<Uuid> {
    top_candidate_ids(
        candidates
            .iter()
            .filter(|candidate| {
                candidate.run_id.is_none() && candidate.status != CandidateStatus::Superseded
            })
            .collect(),
        limit,
    )
    .into_iter()
    .collect()
}

fn top_candidate_ids(mut candidates: Vec<&IdentificationCandidate>, limit: usize) -> Vec<Uuid> {
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.id.cmp(&right.id))
    });

    candidates
        .into_iter()
        .take(limit)
        .map(|candidate| candidate.id)
        .collect()
}

fn should_keep_candidate(
    candidate: &IdentificationCandidate,
    top_candidate_ids: &HashSet<Uuid>,
    legacy_candidate_ids: &HashSet<Uuid>,
) -> bool {
    candidate.status == CandidateStatus::Applied
        || top_candidate_ids.contains(&candidate.id)
        || legacy_candidate_ids.contains(&candidate.id)
}

#[derive(sqlx::FromRow)]
struct CandidateRow {
    id: String,
    book_id: String,
    run_id: Option<String>,
    provider_name: String,
    score: f64,
    metadata: String,
    match_reasons: Option<String>,
    disputes: Option<String>,
    status: String,
    created_at: String,
}

impl CandidateRow {
    fn into_candidate(self) -> Result<IdentificationCandidate, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid candidate UUID: {e}")))?;
        let book_id = Uuid::parse_str(&self.book_id)
            .map_err(|e| DbError::Query(format!("invalid book UUID: {e}")))?;
        let run_id = self
            .run_id
            .map(|value| Uuid::parse_str(&value))
            .transpose()
            .map_err(|e| DbError::Query(format!("invalid resolution run UUID: {e}")))?;
        let metadata: serde_json::Value = serde_json::from_str(&self.metadata)
            .map_err(|e| DbError::Query(format!("invalid metadata JSON: {e}")))?;
        let match_reasons: Vec<String> = self
            .match_reasons
            .as_deref()
            .map(|value| serde_json::from_str(value).unwrap_or_default())
            .unwrap_or_default();
        let disputes: Vec<String> = self
            .disputes
            .as_deref()
            .map(|value| serde_json::from_str(value).unwrap_or_default())
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
            run_id,
            provider_name: self.provider_name,
            score: self.score as f32,
            metadata,
            match_reasons,
            disputes,
            status,
            created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_pool, run_migrations, BookRepository, ResolutionRunRepository};
    use archivis_core::models::{Book, CandidateStatus, ResolutionOutcome, ResolutionRun};
    use chrono::{Duration, Utc};
    use tempfile::TempDir;

    async fn test_pool() -> (SqlitePool, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("candidate-repository.db");
        let pool = create_pool(&db_path).await.unwrap();
        run_migrations(&pool).await.unwrap();
        (pool, dir)
    }

    fn candidate_for(book_id: Uuid, provider: &str, score: f32) -> IdentificationCandidate {
        let mut candidate = IdentificationCandidate::new(
            book_id,
            provider,
            score,
            serde_json::json!({
                "provider_name": provider,
                "title": "Dune",
            }),
            vec!["score".into()],
        );
        candidate.created_at = Utc::now();
        candidate
    }

    #[tokio::test]
    async fn list_by_book_returns_only_latest_reviewable_run_candidates() {
        let (pool, _dir) = test_pool().await;
        let book = Book::new("Dune");
        BookRepository::create(&pool, &book).await.unwrap();

        let mut older_run =
            ResolutionRun::new(book.id, "import", serde_json::json!({"title":"Dune"}));
        older_run.started_at = Utc::now() - Duration::minutes(10);
        older_run.finished_at = Some(Utc::now() - Duration::minutes(9));
        ResolutionRunRepository::create(&pool, &older_run)
            .await
            .unwrap();

        let mut older_a = candidate_for(book.id, "older-a", 0.91);
        older_a.run_id = Some(older_run.id);
        let mut older_b = candidate_for(book.id, "older-b", 0.87);
        older_b.run_id = Some(older_run.id);
        CandidateRepository::create(&pool, &older_a).await.unwrap();
        CandidateRepository::create(&pool, &older_b).await.unwrap();

        let latest_run = ResolutionRunRepository::start(
            &pool,
            book.id,
            "manual_refresh",
            serde_json::json!({"title":"Dune Messiah"}),
            "running",
        )
        .await
        .unwrap();

        let mut latest_a = candidate_for(book.id, "latest-a", 0.97);
        latest_a.run_id = Some(latest_run.id);
        let mut latest_b = candidate_for(book.id, "latest-b", 0.82);
        latest_b.run_id = Some(latest_run.id);
        CandidateRepository::create(&pool, &latest_a).await.unwrap();
        CandidateRepository::create(&pool, &latest_b).await.unwrap();

        CandidateRepository::mark_run_superseded(&pool, older_run.id)
            .await
            .unwrap();

        let current = CandidateRepository::list_by_book(&pool, book.id)
            .await
            .unwrap();
        let current_ids: Vec<Uuid> = current.iter().map(|candidate| candidate.id).collect();
        assert_eq!(current_ids, vec![latest_a.id, latest_b.id]);

        let all = CandidateRepository::list_all_by_book(&pool, book.id)
            .await
            .unwrap();
        assert_eq!(all.len(), 4);
        assert!(all
            .iter()
            .filter(|candidate| candidate.run_id == Some(older_run.id))
            .all(|candidate| candidate.status == CandidateStatus::Superseded));
    }

    #[tokio::test]
    async fn delete_by_book_supersedes_legacy_candidates_instead_of_deleting() {
        let (pool, _dir) = test_pool().await;
        let book = Book::new("Legacy");
        BookRepository::create(&pool, &book).await.unwrap();

        let legacy = candidate_for(book.id, "legacy", 0.73);
        CandidateRepository::create(&pool, &legacy).await.unwrap();

        CandidateRepository::delete_by_book(&pool, book.id)
            .await
            .unwrap();

        let current = CandidateRepository::list_by_book(&pool, book.id)
            .await
            .unwrap();
        assert!(current.is_empty());

        let historical = CandidateRepository::list_all_by_book(&pool, book.id)
            .await
            .unwrap();
        assert_eq!(historical.len(), 1);
        assert_eq!(historical[0].id, legacy.id);
        assert_eq!(historical[0].status, CandidateStatus::Superseded);
    }

    #[tokio::test]
    async fn prune_by_usefulness_keeps_reviewable_top_five_and_applied_candidate() {
        let (pool, _dir) = test_pool().await;
        let book = Book::new("Retention");
        BookRepository::create(&pool, &book).await.unwrap();

        let mut review_run = ResolutionRun::new(
            book.id,
            "manual_refresh",
            serde_json::json!({"title":"Retention"}),
        );
        review_run.state = ResolutionRunState::Done;
        review_run.outcome = Some(ResolutionOutcome::Ambiguous);
        review_run.started_at = Utc::now();
        review_run.finished_at = Some(Utc::now());
        ResolutionRunRepository::create(&pool, &review_run)
            .await
            .unwrap();

        let mut retained_review_ids = Vec::new();
        for score in [0.99_f32, 0.97, 0.95, 0.93, 0.91, 0.12] {
            let mut candidate = candidate_for(book.id, "review", score);
            candidate.run_id = Some(review_run.id);
            retained_review_ids.push(candidate.id);
            CandidateRepository::create(&pool, &candidate)
                .await
                .unwrap();
        }

        let mut confirmed_run = ResolutionRun::new(
            book.id,
            "manual_refresh",
            serde_json::json!({"title":"Retention"}),
        );
        confirmed_run.state = ResolutionRunState::Done;
        confirmed_run.outcome = Some(ResolutionOutcome::Confirmed);
        confirmed_run.started_at = Utc::now() - Duration::minutes(5);
        confirmed_run.finished_at = Some(Utc::now() - Duration::minutes(4));
        ResolutionRunRepository::create(&pool, &confirmed_run)
            .await
            .unwrap();

        let mut applied = candidate_for(book.id, "confirmed-applied", 0.88);
        applied.run_id = Some(confirmed_run.id);
        applied.status = CandidateStatus::Applied;
        CandidateRepository::create(&pool, &applied).await.unwrap();

        let mut confirmed_pending = candidate_for(book.id, "confirmed-pending", 0.67);
        confirmed_pending.run_id = Some(confirmed_run.id);
        CandidateRepository::create(&pool, &confirmed_pending)
            .await
            .unwrap();

        let mut unmatched_run = ResolutionRun::new(
            book.id,
            "manual_refresh",
            serde_json::json!({"title":"Retention"}),
        );
        unmatched_run.state = ResolutionRunState::Done;
        unmatched_run.outcome = Some(ResolutionOutcome::Unmatched);
        unmatched_run.started_at = Utc::now() - Duration::minutes(10);
        unmatched_run.finished_at = Some(Utc::now() - Duration::minutes(9));
        ResolutionRunRepository::create(&pool, &unmatched_run)
            .await
            .unwrap();

        let mut unmatched = candidate_for(book.id, "unmatched", 0.52);
        unmatched.run_id = Some(unmatched_run.id);
        CandidateRepository::create(&pool, &unmatched)
            .await
            .unwrap();

        CandidateRepository::prune_by_usefulness(&pool, book.id)
            .await
            .unwrap();

        let remaining = CandidateRepository::list_all_by_book(&pool, book.id)
            .await
            .unwrap();
        let remaining_ids: HashSet<Uuid> = remaining.iter().map(|candidate| candidate.id).collect();

        assert_eq!(remaining.len(), 6);
        assert!(retained_review_ids
            .into_iter()
            .take(5)
            .all(|candidate_id| remaining_ids.contains(&candidate_id)));
        assert!(!remaining_ids.contains(&confirmed_pending.id));
        assert!(!remaining_ids.contains(&unmatched.id));
        assert!(remaining_ids.contains(&applied.id));
    }

    #[tokio::test]
    async fn prune_by_usefulness_keeps_only_last_two_superseded_review_runs() {
        let (pool, _dir) = test_pool().await;
        let book = Book::new("Superseded");
        BookRepository::create(&pool, &book).await.unwrap();

        let mut oldest = ResolutionRun::new(
            book.id,
            "manual_refresh",
            serde_json::json!({"title":"Superseded"}),
        );
        oldest.state = ResolutionRunState::Superseded;
        oldest.outcome = Some(ResolutionOutcome::Disputed);
        oldest.started_at = Utc::now() - Duration::minutes(30);
        oldest.finished_at = Some(Utc::now() - Duration::minutes(29));
        ResolutionRunRepository::create(&pool, &oldest)
            .await
            .unwrap();

        let mut middle = ResolutionRun::new(
            book.id,
            "manual_refresh",
            serde_json::json!({"title":"Superseded"}),
        );
        middle.state = ResolutionRunState::Superseded;
        middle.outcome = Some(ResolutionOutcome::Disputed);
        middle.started_at = Utc::now() - Duration::minutes(20);
        middle.finished_at = Some(Utc::now() - Duration::minutes(19));
        ResolutionRunRepository::create(&pool, &middle)
            .await
            .unwrap();

        let mut latest = ResolutionRun::new(
            book.id,
            "manual_refresh",
            serde_json::json!({"title":"Superseded"}),
        );
        latest.state = ResolutionRunState::Superseded;
        latest.outcome = Some(ResolutionOutcome::Disputed);
        latest.started_at = Utc::now() - Duration::minutes(10);
        latest.finished_at = Some(Utc::now() - Duration::minutes(9));
        ResolutionRunRepository::create(&pool, &latest)
            .await
            .unwrap();

        let mut oldest_candidate = candidate_for(book.id, "oldest", 0.9);
        oldest_candidate.run_id = Some(oldest.id);
        oldest_candidate.status = CandidateStatus::Superseded;
        CandidateRepository::create(&pool, &oldest_candidate)
            .await
            .unwrap();

        let mut middle_candidate = candidate_for(book.id, "middle", 0.9);
        middle_candidate.run_id = Some(middle.id);
        middle_candidate.status = CandidateStatus::Superseded;
        CandidateRepository::create(&pool, &middle_candidate)
            .await
            .unwrap();

        let mut latest_candidate = candidate_for(book.id, "latest", 0.9);
        latest_candidate.run_id = Some(latest.id);
        latest_candidate.status = CandidateStatus::Superseded;
        CandidateRepository::create(&pool, &latest_candidate)
            .await
            .unwrap();

        CandidateRepository::prune_by_usefulness(&pool, book.id)
            .await
            .unwrap();

        let remaining = CandidateRepository::list_all_by_book(&pool, book.id)
            .await
            .unwrap();
        let remaining_ids: HashSet<Uuid> = remaining.iter().map(|candidate| candidate.id).collect();

        assert_eq!(remaining.len(), 2);
        assert!(remaining_ids.contains(&middle_candidate.id));
        assert!(remaining_ids.contains(&latest_candidate.id));
        assert!(!remaining_ids.contains(&oldest_candidate.id));
    }
}
