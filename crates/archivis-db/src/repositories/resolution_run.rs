use archivis_core::errors::DbError;
use archivis_core::models::{ResolutionOutcome, ResolutionRun, ResolutionRunState};
use chrono::{DateTime, Utc};
use sqlx::{FromRow, SqliteConnection, SqlitePool};
use uuid::Uuid;

use crate::CandidateRepository;

const MAX_RETAINED_RESOLUTION_RUNS_PER_BOOK: i64 = 10;

pub struct ResolutionRunRepository;

impl ResolutionRunRepository {
    pub async fn create(pool: &SqlitePool, run: &ResolutionRun) -> Result<(), DbError> {
        let best_candidate_id = run.best_candidate_id.as_ref().map(ToString::to_string);
        let started_at = run.started_at.to_rfc3339();
        let finished_at = run.finished_at.as_ref().map(DateTime::<Utc>::to_rfc3339);

        sqlx::query(
            "INSERT INTO resolution_runs (
                 id, book_id, trigger, state, outcome, query_json, decision_code,
                 candidate_count, best_candidate_id, best_score, best_tier, error,
                 started_at, finished_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run.id.to_string())
        .bind(run.book_id.to_string())
        .bind(&run.trigger)
        .bind(run.state.to_string())
        .bind(serialize_outcome(run.outcome))
        .bind(serialize_json(&run.query_json)?)
        .bind(&run.decision_code)
        .bind(run.candidate_count)
        .bind(best_candidate_id)
        .bind(run.best_score)
        .bind(run.best_tier.as_deref())
        .bind(run.error.as_deref())
        .bind(started_at)
        .bind(finished_at)
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn start(
        pool: &SqlitePool,
        book_id: Uuid,
        trigger: &str,
        query_json: serde_json::Value,
        decision_code: &str,
    ) -> Result<ResolutionRun, DbError> {
        let mut run = ResolutionRun::new(book_id, trigger, query_json);
        run.decision_code = decision_code.into();
        Self::create(pool, &run).await?;
        Self::set_book_current_run(pool, run.book_id, run.id).await?;
        Ok(run)
    }

    pub async fn get_by_id(
        pool: &SqlitePool,
        run_id: Uuid,
    ) -> Result<Option<ResolutionRun>, DbError> {
        let row = sqlx::query_as::<_, ResolutionRunRow>(
            "SELECT id, book_id, trigger, state, outcome, query_json, decision_code,
                    candidate_count, best_candidate_id, best_score, best_tier, error,
                    started_at, finished_at
             FROM resolution_runs
             WHERE id = ?",
        )
        .bind(run_id.to_string())
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        row.map(ResolutionRunRow::into_run).transpose()
    }

    pub async fn list_by_book(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Vec<ResolutionRun>, DbError> {
        let rows = sqlx::query_as::<_, ResolutionRunRow>(
            "SELECT id, book_id, trigger, state, outcome, query_json, decision_code,
                    candidate_count, best_candidate_id, best_score, best_tier, error,
                    started_at, finished_at
             FROM resolution_runs
             WHERE book_id = ?
             ORDER BY started_at DESC",
        )
        .bind(book_id.to_string())
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(ResolutionRunRow::into_run).collect()
    }

    pub async fn list_running(pool: &SqlitePool) -> Result<Vec<ResolutionRun>, DbError> {
        let rows = sqlx::query_as::<_, ResolutionRunRow>(
            "SELECT id, book_id, trigger, state, outcome, query_json, decision_code,
                    candidate_count, best_candidate_id, best_score, best_tier, error,
                    started_at, finished_at
             FROM resolution_runs
             WHERE state = 'running'
             ORDER BY started_at ASC",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(ResolutionRunRow::into_run).collect()
    }

    pub async fn latest_for_book(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Option<ResolutionRun>, DbError> {
        let row = sqlx::query_as::<_, ResolutionRunRow>(
            "SELECT id, book_id, trigger, state, outcome, query_json, decision_code,
                    candidate_count, best_candidate_id, best_score, best_tier, error,
                    started_at, finished_at
             FROM resolution_runs
             WHERE book_id = ?
             ORDER BY started_at DESC
             LIMIT 1",
        )
        .bind(book_id.to_string())
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        row.map(ResolutionRunRow::into_run).transpose()
    }

    pub async fn latest_by_book(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Option<ResolutionRun>, DbError> {
        Self::latest_for_book(pool, book_id).await
    }

    pub async fn current_for_book(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Option<ResolutionRun>, DbError> {
        let row = sqlx::query_as::<_, ResolutionRunRow>(
            "SELECT rr.id, rr.book_id, rr.trigger, rr.state, rr.outcome, rr.query_json,
                    rr.decision_code, rr.candidate_count, rr.best_candidate_id, rr.best_score,
                    rr.best_tier, rr.error, rr.started_at, rr.finished_at
             FROM books b
             JOIN resolution_runs rr ON rr.id = b.last_resolution_run_id
             WHERE b.id = ?
               AND rr.state != 'superseded'
             LIMIT 1",
        )
        .bind(book_id.to_string())
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if let Some(row) = row {
            return row.into_run().map(Some);
        }

        let row = sqlx::query_as::<_, ResolutionRunRow>(
            "SELECT id, book_id, trigger, state, outcome, query_json, decision_code,
                    candidate_count, best_candidate_id, best_score, best_tier, error,
                    started_at, finished_at
             FROM resolution_runs
             WHERE book_id = ?
               AND state != 'superseded'
             ORDER BY started_at DESC
             LIMIT 1",
        )
        .bind(book_id.to_string())
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        row.map(ResolutionRunRow::into_run).transpose()
    }

    pub async fn update(pool: &SqlitePool, run: &ResolutionRun) -> Result<(), DbError> {
        let result = sqlx::query(
            "UPDATE resolution_runs
             SET book_id = ?,
                 trigger = ?,
                 state = ?,
                 outcome = ?,
                 query_json = ?,
                 decision_code = ?,
                 candidate_count = ?,
                 best_candidate_id = ?,
                 best_score = ?,
                 best_tier = ?,
                 error = ?,
                 started_at = ?,
                 finished_at = ?
             WHERE id = ?",
        )
        .bind(run.book_id.to_string())
        .bind(&run.trigger)
        .bind(run.state.to_string())
        .bind(serialize_outcome(run.outcome))
        .bind(serialize_json(&run.query_json)?)
        .bind(&run.decision_code)
        .bind(run.candidate_count)
        .bind(run.best_candidate_id.as_ref().map(ToString::to_string))
        .bind(run.best_score)
        .bind(run.best_tier.as_deref())
        .bind(run.error.as_deref())
        .bind(run.started_at.to_rfc3339())
        .bind(run.finished_at.as_ref().map(DateTime::<Utc>::to_rfc3339))
        .bind(run.id.to_string())
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "resolution_run",
                id: run.id.to_string(),
            });
        }

        Ok(())
    }

    pub async fn finalize(pool: &SqlitePool, run: &ResolutionRun) -> Result<(), DbError> {
        if run.state == ResolutionRunState::Running {
            return Err(DbError::Constraint(
                "cannot finalize resolution run while still running".into(),
            ));
        }
        if run.finished_at.is_none() {
            return Err(DbError::Constraint(
                "cannot finalize resolution run without finished_at".into(),
            ));
        }

        Self::update(pool, run).await?;
        Self::set_book_current_run(pool, run.book_id, run.id).await?;
        CandidateRepository::prune_by_usefulness(pool, run.book_id).await?;
        Self::prune_run_history(pool, run.book_id).await?;
        Ok(())
    }

    pub async fn mark_older_runs_superseded(
        pool: &SqlitePool,
        book_id: Uuid,
        keep_run_id: Uuid,
    ) -> Result<u64, DbError> {
        let finished_at = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE resolution_runs
             SET state = 'superseded',
                 finished_at = COALESCE(finished_at, ?)
             WHERE book_id = ?
               AND id != ?
               AND state != 'superseded'
               AND EXISTS (
                   SELECT 1
                   FROM identification_candidates candidates
                   WHERE candidates.run_id = resolution_runs.id
                     AND candidates.status != 'superseded'
               )",
        )
        .bind(finished_at)
        .bind(book_id.to_string())
        .bind(keep_run_id.to_string())
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(result.rows_affected())
    }

    /// Mark a single resolution run as `superseded`, setting `finished_at`
    /// if it was not already set. Returns `Ok(true)` if the row was updated.
    pub async fn supersede_run_conn(
        conn: &mut SqliteConnection,
        run_id: Uuid,
    ) -> Result<bool, DbError> {
        let finished_at = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE resolution_runs
             SET state = 'superseded',
                 finished_at = COALESCE(finished_at, ?)
             WHERE id = ? AND state != 'superseded'",
        )
        .bind(&finished_at)
        .bind(run_id.to_string())
        .execute(&mut *conn)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    async fn set_book_current_run(
        pool: &SqlitePool,
        book_id: Uuid,
        run_id: Uuid,
    ) -> Result<(), DbError> {
        let result = sqlx::query(
            "UPDATE books
             SET last_resolution_run_id = ?,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(run_id.to_string())
        .bind(Utc::now().to_rfc3339())
        .bind(book_id.to_string())
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "book",
                id: book_id.to_string(),
            });
        }

        Ok(())
    }

    async fn prune_run_history(pool: &SqlitePool, book_id: Uuid) -> Result<u64, DbError> {
        let current_run_id = Self::book_current_run_id(pool, book_id).await?;
        let retained_other_runs = if current_run_id.is_some() {
            MAX_RETAINED_RESOLUTION_RUNS_PER_BOOK.saturating_sub(1)
        } else {
            MAX_RETAINED_RESOLUTION_RUNS_PER_BOOK
        };

        let run_ids_to_delete = if let Some(current_run_id) = current_run_id {
            sqlx::query_scalar::<_, String>(
                "SELECT id
                 FROM resolution_runs
                 WHERE book_id = ?
                   AND id != ?
                 ORDER BY started_at DESC
                 LIMIT -1 OFFSET ?",
            )
            .bind(book_id.to_string())
            .bind(current_run_id)
            .bind(retained_other_runs)
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?
        } else {
            sqlx::query_scalar::<_, String>(
                "SELECT id
                 FROM resolution_runs
                 WHERE book_id = ?
                 ORDER BY started_at DESC
                 LIMIT -1 OFFSET ?",
            )
            .bind(book_id.to_string())
            .bind(retained_other_runs)
            .fetch_all(pool)
            .await
            .map_err(|e| DbError::Query(e.to_string()))?
        };

        let mut deleted_runs = 0;
        for run_id in run_ids_to_delete {
            sqlx::query("DELETE FROM identification_candidates WHERE run_id = ?")
                .bind(&run_id)
                .execute(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?;

            deleted_runs += sqlx::query("DELETE FROM resolution_runs WHERE id = ?")
                .bind(&run_id)
                .execute(pool)
                .await
                .map_err(|e| DbError::Query(e.to_string()))?
                .rows_affected();
        }

        Ok(deleted_runs)
    }

    async fn book_current_run_id(
        pool: &SqlitePool,
        book_id: Uuid,
    ) -> Result<Option<String>, DbError> {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT last_resolution_run_id
             FROM books
             WHERE id = ?",
        )
        .bind(book_id.to_string())
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))
    }
}

fn serialize_outcome(outcome: Option<ResolutionOutcome>) -> Option<String> {
    outcome.map(|value| value.to_string())
}

fn serialize_json(value: &serde_json::Value) -> Result<String, DbError> {
    serde_json::to_string(value)
        .map_err(|e| DbError::Query(format!("failed to serialize JSON: {e}")))
}

fn parse_datetime(value: &str, field: &str) -> Result<DateTime<Utc>, DbError> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.fZ")
                .map(|dt| dt.and_utc())
        })
        .map_err(|e| DbError::Query(format!("invalid {field}: {e}")))
}

#[derive(Debug, FromRow)]
struct ResolutionRunRow {
    id: String,
    book_id: String,
    trigger: String,
    state: String,
    outcome: Option<String>,
    query_json: String,
    decision_code: String,
    candidate_count: i64,
    best_candidate_id: Option<String>,
    best_score: Option<f64>,
    best_tier: Option<String>,
    error: Option<String>,
    started_at: String,
    finished_at: Option<String>,
}

impl ResolutionRunRow {
    fn into_run(self) -> Result<ResolutionRun, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid resolution_run UUID: {e}")))?;
        let book_id = Uuid::parse_str(&self.book_id)
            .map_err(|e| DbError::Query(format!("invalid resolution_run book UUID: {e}")))?;
        let state = self
            .state
            .parse::<ResolutionRunState>()
            .map_err(DbError::Query)?;
        let outcome = self
            .outcome
            .map(|value| value.parse::<ResolutionOutcome>())
            .transpose()
            .map_err(DbError::Query)?;
        let query_json = serde_json::from_str(&self.query_json)
            .map_err(|e| DbError::Query(format!("invalid query_json: {e}")))?;
        let best_candidate_id = self
            .best_candidate_id
            .map(|value| Uuid::parse_str(&value))
            .transpose()
            .map_err(|e| DbError::Query(format!("invalid best_candidate_id: {e}")))?;
        let started_at = parse_datetime(&self.started_at, "started_at")?;
        let finished_at = self
            .finished_at
            .as_deref()
            .map(|value| parse_datetime(value, "finished_at"))
            .transpose()?;

        #[allow(clippy::cast_possible_truncation)]
        Ok(ResolutionRun {
            id,
            book_id,
            trigger: self.trigger,
            state,
            outcome,
            query_json,
            decision_code: self.decision_code,
            candidate_count: self.candidate_count,
            best_candidate_id,
            best_score: self.best_score.map(|value| value as f32),
            best_tier: self.best_tier,
            error: self.error,
            started_at,
            finished_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_pool, run_migrations, BookRepository, CandidateRepository};
    use archivis_core::models::{Book, IdentificationCandidate};
    use tempfile::TempDir;

    async fn test_pool() -> (SqlitePool, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let pool = create_pool(&db_path).await.unwrap();
        run_migrations(&pool).await.unwrap();
        (pool, dir)
    }

    #[tokio::test]
    async fn start_and_finalize_run_round_trip() {
        let (pool, _dir) = test_pool().await;
        let book = Book::new("Run Test");
        BookRepository::create(&pool, &book).await.unwrap();

        let mut run = ResolutionRunRepository::start(
            &pool,
            book.id,
            "manual_refresh",
            serde_json::json!({"title":"Run Test"}),
            "running",
        )
        .await
        .unwrap();

        let current = ResolutionRunRepository::current_for_book(&pool, book.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.id, run.id);
        assert_eq!(current.state, ResolutionRunState::Running);

        run.state = ResolutionRunState::Done;
        run.outcome = Some(ResolutionOutcome::Unmatched);
        run.decision_code = "no_candidates".into();
        run.finished_at = Some(Utc::now());

        ResolutionRunRepository::finalize(&pool, &run)
            .await
            .unwrap();

        let stored = ResolutionRunRepository::get_by_id(&pool, run.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.state, ResolutionRunState::Done);
        assert_eq!(stored.outcome, Some(ResolutionOutcome::Unmatched));
        assert_eq!(stored.decision_code, "no_candidates");
        assert!(stored.finished_at.is_some());

        let book_after = BookRepository::get_by_id(&pool, book.id).await.unwrap();
        assert_eq!(book_after.last_resolution_run_id, Some(run.id));
    }

    #[tokio::test]
    async fn current_for_book_falls_back_to_latest_non_superseded_run() {
        let (pool, _dir) = test_pool().await;
        let book = Book::new("Fallback");
        BookRepository::create(&pool, &book).await.unwrap();

        let mut older =
            ResolutionRun::new(book.id, "import", serde_json::json!({"title":"Fallback"}));
        older.state = ResolutionRunState::Done;
        older.started_at = Utc::now() - chrono::Duration::minutes(10);
        older.finished_at = Some(Utc::now() - chrono::Duration::minutes(9));
        ResolutionRunRepository::create(&pool, &older)
            .await
            .unwrap();

        let mut latest = ResolutionRun::new(
            book.id,
            "manual_refresh",
            serde_json::json!({"title":"Fallback"}),
        );
        latest.state = ResolutionRunState::Superseded;
        latest.finished_at = Some(Utc::now());
        ResolutionRunRepository::create(&pool, &latest)
            .await
            .unwrap();

        let current = ResolutionRunRepository::current_for_book(&pool, book.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.id, older.id);

        let latest_any = ResolutionRunRepository::latest_for_book(&pool, book.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest_any.id, latest.id);
    }

    #[tokio::test]
    async fn finalize_prunes_review_candidates_down_to_top_five() {
        let (pool, _dir) = test_pool().await;
        let book = Book::new("Retention");
        BookRepository::create(&pool, &book).await.unwrap();

        let mut review_run =
            ResolutionRun::new(book.id, "import", serde_json::json!({"title":"Retention"}));
        review_run.state = ResolutionRunState::Done;
        review_run.outcome = Some(ResolutionOutcome::Disputed);
        review_run.started_at = Utc::now() - chrono::Duration::minutes(10);
        review_run.finished_at = Some(Utc::now() - chrono::Duration::minutes(9));
        ResolutionRunRepository::create(&pool, &review_run)
            .await
            .unwrap();

        let mut retained_ids = Vec::new();
        for (provider_name, score) in [
            ("provider-a", 0.99_f32),
            ("provider-b", 0.97),
            ("provider-c", 0.95),
            ("provider-d", 0.93),
            ("provider-e", 0.91),
            ("provider-f", 0.12),
        ] {
            let mut candidate = IdentificationCandidate::new(
                book.id,
                provider_name,
                score,
                serde_json::json!({"title":"Retention"}),
                vec!["title_match".into()],
            );
            candidate.run_id = Some(review_run.id);
            retained_ids.push(candidate.id);
            CandidateRepository::create(&pool, &candidate)
                .await
                .unwrap();
        }

        let mut current_run = ResolutionRunRepository::start(
            &pool,
            book.id,
            "manual_refresh",
            serde_json::json!({"title":"Retention"}),
            "running",
        )
        .await
        .unwrap();
        current_run.state = ResolutionRunState::Done;
        current_run.outcome = Some(ResolutionOutcome::Enriched);
        current_run.finished_at = Some(Utc::now());

        ResolutionRunRepository::finalize(&pool, &current_run)
            .await
            .unwrap();

        let remaining = CandidateRepository::list_all_by_book(&pool, book.id)
            .await
            .unwrap();
        let remaining_ids: std::collections::HashSet<Uuid> =
            remaining.iter().map(|candidate| candidate.id).collect();

        assert_eq!(remaining_ids.len(), 5);
        assert!(retained_ids
            .into_iter()
            .take(5)
            .all(|candidate_id| remaining_ids.contains(&candidate_id)));
    }

    #[tokio::test]
    async fn finalize_caps_run_history_per_book() {
        let (pool, _dir) = test_pool().await;
        let book = Book::new("Run Cap");
        BookRepository::create(&pool, &book).await.unwrap();

        for minutes_ago in 1..=12 {
            let mut historical_run =
                ResolutionRun::new(book.id, "import", serde_json::json!({"title":"Run Cap"}));
            historical_run.state = ResolutionRunState::Done;
            historical_run.outcome = Some(ResolutionOutcome::Unmatched);
            historical_run.started_at =
                Utc::now() - chrono::Duration::minutes(i64::from(minutes_ago));
            historical_run.finished_at =
                Some(historical_run.started_at + chrono::Duration::seconds(30));
            ResolutionRunRepository::create(&pool, &historical_run)
                .await
                .unwrap();
        }

        let mut current_run = ResolutionRunRepository::start(
            &pool,
            book.id,
            "manual_refresh",
            serde_json::json!({"title":"Run Cap"}),
            "running",
        )
        .await
        .unwrap();
        current_run.state = ResolutionRunState::Done;
        current_run.outcome = Some(ResolutionOutcome::Confirmed);
        current_run.finished_at = Some(Utc::now());

        ResolutionRunRepository::finalize(&pool, &current_run)
            .await
            .unwrap();

        let retained = ResolutionRunRepository::list_by_book(&pool, book.id)
            .await
            .unwrap();
        assert_eq!(
            retained.len(),
            usize::try_from(MAX_RETAINED_RESOLUTION_RUNS_PER_BOOK).unwrap()
        );
        assert!(retained.iter().any(|run| run.id == current_run.id));
    }
}
