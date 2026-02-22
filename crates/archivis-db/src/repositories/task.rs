use archivis_core::errors::DbError;
use archivis_core::models::{Task, TaskStatus, TaskType};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct TaskRepository;

impl TaskRepository {
    pub async fn create(pool: &SqlitePool, task: &Task) -> Result<(), DbError> {
        let id = task.id.to_string();
        let task_type = task.task_type.to_string();
        let payload = task.payload.to_string();
        let status = task.status.to_string();
        let progress = i64::from(task.progress);
        let created_at = task.created_at.to_rfc3339();
        let started_at = task.started_at.map(|t| t.to_rfc3339());
        let completed_at = task.completed_at.map(|t| t.to_rfc3339());
        let result = task.result.as_ref().map(ToString::to_string);

        sqlx::query!(
            "INSERT INTO tasks (id, task_type, payload, status, progress, message, result, created_at, started_at, completed_at, error_message)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            id,
            task_type,
            payload,
            status,
            progress,
            task.message,
            result,
            created_at,
            started_at,
            completed_at,
            task.error_message,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        Ok(())
    }

    pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Task, DbError> {
        let id_str = id.to_string();
        let row = sqlx::query_as!(
            TaskRow,
            "SELECT id, task_type, payload, status, progress, message, result, created_at, started_at, completed_at, error_message FROM tasks WHERE id = ?",
            id_str,
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?
        .ok_or(DbError::NotFound {
            entity: "task",
            id: id_str,
        })?;

        row.into_task()
    }

    /// List tasks that are still active (pending or running).
    pub async fn list_active(pool: &SqlitePool) -> Result<Vec<Task>, DbError> {
        let rows = sqlx::query_as!(
            TaskRow,
            "SELECT id, task_type, payload, status, progress, message, result, created_at, started_at, completed_at, error_message FROM tasks WHERE status IN ('pending', 'running') ORDER BY created_at ASC",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(TaskRow::into_task).collect()
    }

    /// List the most recent tasks, regardless of status.
    pub async fn list_recent(pool: &SqlitePool, limit: i64) -> Result<Vec<Task>, DbError> {
        let rows = sqlx::query_as!(
            TaskRow,
            "SELECT id, task_type, payload, status, progress, message, result, created_at, started_at, completed_at, error_message FROM tasks ORDER BY created_at DESC LIMIT ?",
            limit,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(TaskRow::into_task).collect()
    }

    /// Update a task's status and optional timestamp/error fields.
    pub async fn update_status(
        pool: &SqlitePool,
        id: Uuid,
        status: TaskStatus,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
        error_message: Option<&str>,
        result: Option<&serde_json::Value>,
    ) -> Result<(), DbError> {
        let id_str = id.to_string();
        let status_str = status.to_string();
        let started_str = started_at.map(|t| t.to_rfc3339());
        let completed_str = completed_at.map(|t| t.to_rfc3339());
        let result_str = result.map(ToString::to_string);

        let affected = sqlx::query!(
            "UPDATE tasks SET status = ?, started_at = COALESCE(?, started_at), completed_at = COALESCE(?, completed_at), error_message = COALESCE(?, error_message), result = COALESCE(?, result) WHERE id = ?",
            status_str,
            started_str,
            completed_str,
            error_message,
            result_str,
            id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if affected.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "task",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Update a task's progress percentage and optional message.
    pub async fn update_progress(
        pool: &SqlitePool,
        id: Uuid,
        progress: u8,
        message: Option<&str>,
    ) -> Result<(), DbError> {
        let id_str = id.to_string();
        let progress_val = i64::from(progress);

        let affected = sqlx::query!(
            "UPDATE tasks SET progress = ?, message = COALESCE(?, message) WHERE id = ?",
            progress_val,
            message,
            id_str,
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        if affected.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "task",
                id: id_str,
            });
        }

        Ok(())
    }

    /// Recover tasks that were running when the application was interrupted.
    /// Resets them to pending so they can be re-dispatched.
    /// Returns the recovered tasks.
    pub async fn recover_interrupted(pool: &SqlitePool) -> Result<Vec<Task>, DbError> {
        sqlx::query!(
            "UPDATE tasks SET status = 'pending', started_at = NULL, progress = 0, message = 'recovered after restart' WHERE status = 'running'",
        )
        .execute(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        // Return all pending tasks (includes both previously pending and just-recovered)
        let rows = sqlx::query_as!(
            TaskRow,
            "SELECT id, task_type, payload, status, progress, message, result, created_at, started_at, completed_at, error_message FROM tasks WHERE status = 'pending' ORDER BY created_at ASC",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;

        rows.into_iter().map(TaskRow::into_task).collect()
    }
}

// ── Row type for sqlx mapping ──────────────────────────────────

#[derive(sqlx::FromRow)]
struct TaskRow {
    id: String,
    task_type: String,
    payload: String,
    status: String,
    progress: i64,
    message: Option<String>,
    result: Option<String>,
    created_at: String,
    started_at: Option<String>,
    completed_at: Option<String>,
    error_message: Option<String>,
}

impl TaskRow {
    fn into_task(self) -> Result<Task, DbError> {
        let id = Uuid::parse_str(&self.id)
            .map_err(|e| DbError::Query(format!("invalid task UUID: {e}")))?;
        let task_type: TaskType = self
            .task_type
            .parse()
            .map_err(|e: String| DbError::Query(e))?;
        let payload: serde_json::Value = serde_json::from_str(&self.payload)
            .map_err(|e| DbError::Query(format!("invalid task payload JSON: {e}")))?;
        let status: TaskStatus = self.status.parse().map_err(|e: String| DbError::Query(e))?;
        let created_at = parse_datetime(&self.created_at, "created_at")?;
        let started_at = self
            .started_at
            .map(|s| parse_datetime(&s, "started_at"))
            .transpose()?;
        let completed_at = self
            .completed_at
            .map(|s| parse_datetime(&s, "completed_at"))
            .transpose()?;
        let result: Option<serde_json::Value> = self
            .result
            .map(|r| serde_json::from_str(&r))
            .transpose()
            .map_err(|e| DbError::Query(format!("invalid task result JSON: {e}")))?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(Task {
            id,
            task_type,
            payload,
            status,
            progress: self.progress as u8,
            message: self.message,
            result,
            created_at,
            started_at,
            completed_at,
            error_message: self.error_message,
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
