pub mod cancellation;
mod worker;

pub use cancellation::CancellationRegistry;
pub use worker::{ProgressSender, Worker};

use std::collections::HashMap;
use std::sync::Arc;

use archivis_core::errors::TaskError;
use archivis_core::models::{ResolutionRunState, Task, TaskProgress, TaskStatus, TaskType};
use archivis_db::{
    BookRepository, CandidateRepository, DbPool, ResolutionRunRepository, TaskRepository,
};
use chrono::Utc;
use tokio::sync::{broadcast, mpsc};

/// In-memory task queue backed by `SQLite` for persistence.
///
/// Tasks are enqueued into a database table and dispatched to registered workers
/// via an internal channel. Progress updates are broadcast to SSE subscribers.
pub struct TaskQueue {
    db_pool: DbPool,
    dispatch_tx: mpsc::Sender<Task>,
    progress_tx: broadcast::Sender<TaskProgress>,
    cancellation_registry: Arc<CancellationRegistry>,
}

impl TaskQueue {
    /// Create a new task queue.
    ///
    /// Returns the queue and an `mpsc::Receiver` that the dispatcher loop should
    /// consume via [`run_dispatcher`].
    pub fn new(db_pool: DbPool) -> (Self, mpsc::Receiver<Task>) {
        let (dispatch_tx, dispatch_rx) = mpsc::channel(256);
        let (progress_tx, _) = broadcast::channel(1024);

        let queue = Self {
            db_pool,
            dispatch_tx,
            progress_tx,
            cancellation_registry: Arc::new(CancellationRegistry::new()),
        };

        (queue, dispatch_rx)
    }

    /// Enqueue a new task. Persists it to the database and sends it for dispatch.
    pub async fn enqueue(
        &self,
        task_type: TaskType,
        payload: serde_json::Value,
    ) -> Result<uuid::Uuid, TaskError> {
        let task = Task::new(task_type, payload);
        let task_id = task.id;

        TaskRepository::create(&self.db_pool, &task)
            .await
            .map_err(|e| TaskError::Internal(e.to_string()))?;

        self.dispatch_tx
            .send(task)
            .await
            .map_err(|_| TaskError::QueueFull)?;

        tracing::info!(%task_id, %task_type, "task enqueued");
        Ok(task_id)
    }

    /// Enqueue a child task linked to a parent. Persists it and sends for dispatch.
    pub async fn enqueue_child(
        &self,
        task_type: TaskType,
        payload: serde_json::Value,
        parent_id: uuid::Uuid,
    ) -> Result<uuid::Uuid, TaskError> {
        let task = Task::new_child(task_type, payload, parent_id);
        let task_id = task.id;

        TaskRepository::create(&self.db_pool, &task)
            .await
            .map_err(|e| TaskError::Internal(e.to_string()))?;

        self.dispatch_tx
            .send(task)
            .await
            .map_err(|_| TaskError::QueueFull)?;

        tracing::info!(%task_id, %task_type, %parent_id, "child task enqueued");
        Ok(task_id)
    }

    /// Subscribe to progress updates for all tasks.
    pub fn subscribe_all(&self) -> broadcast::Receiver<TaskProgress> {
        self.progress_tx.subscribe()
    }

    /// Create a `ProgressSender` that workers use to report progress.
    pub fn progress_sender(&self) -> ProgressSender {
        ProgressSender::new(self.progress_tx.clone(), self.db_pool.clone())
    }

    /// Get a reference to the database pool.
    pub fn db_pool(&self) -> &DbPool {
        &self.db_pool
    }

    /// Get a clone of the dispatch sender (useful for re-enqueuing recovered tasks).
    pub fn dispatch_sender(&self) -> mpsc::Sender<Task> {
        self.dispatch_tx.clone()
    }

    /// Get the cancellation registry.
    pub fn cancellation_registry(&self) -> &Arc<CancellationRegistry> {
        &self.cancellation_registry
    }
}

/// Run the task dispatcher loop.
///
/// Receives tasks from the channel and dispatches them to the appropriate worker.
/// This function runs until the channel is closed (all senders dropped).
pub async fn run_dispatcher<S: ::std::hash::BuildHasher>(
    mut rx: mpsc::Receiver<Task>,
    workers: HashMap<TaskType, Arc<dyn Worker>, S>,
    progress: ProgressSender,
    db_pool: DbPool,
    cancellation_registry: Arc<CancellationRegistry>,
) {
    tracing::info!(worker_count = workers.len(), "task dispatcher started",);

    while let Some(task) = rx.recv().await {
        let task_id = task.id;
        let task_type = task.task_type;

        // Skip cancelled tasks (e.g. pending children that were cancelled before dispatch)
        if task.status == TaskStatus::Cancelled {
            tracing::debug!(%task_id, "skipping already-cancelled task");
            continue;
        }

        let Some(worker) = workers.get(&task_type) else {
            tracing::error!(%task_id, %task_type, "no worker registered for task type");
            let _ = TaskRepository::update_status(
                &db_pool,
                task_id,
                TaskStatus::Failed,
                None,
                Some(Utc::now()),
                Some(&format!("no worker registered for task type: {task_type}")),
                None,
            )
            .await;
            continue;
        };

        let worker = Arc::clone(worker);
        let token = cancellation_registry.register(task_id);
        let progress = progress
            .for_task(task_id)
            .with_task_type(task.task_type)
            .with_parent(task.parent_task_id)
            .with_cancellation_token(token);
        let pool = db_pool.clone();
        let registry = Arc::clone(&cancellation_registry);

        tokio::spawn(async move {
            dispatch_task(
                task_id,
                task.parent_task_id,
                task.payload,
                &*worker,
                progress,
                &pool,
            )
            .await;
            registry.remove(task_id);
        });
    }

    tracing::info!("task dispatcher stopped");
}

/// Execute a single task: mark running, call worker, mark completed/failed/cancelled.
#[allow(clippy::too_many_lines)]
async fn dispatch_task(
    task_id: uuid::Uuid,
    parent_task_id: Option<uuid::Uuid>,
    payload: serde_json::Value,
    worker: &dyn Worker,
    progress: ProgressSender,
    pool: &DbPool,
) {
    let now = Utc::now();

    // Mark as running
    if let Err(e) = TaskRepository::update_status(
        pool,
        task_id,
        TaskStatus::Running,
        Some(now),
        None,
        None,
        None,
    )
    .await
    {
        tracing::error!(%task_id, error = %e, "failed to mark task as running");
        return;
    }

    // Execute worker
    match worker.execute(payload, progress.clone()).await {
        Ok(result) => {
            let completed_at = Utc::now();
            if let Err(e) = TaskRepository::update_status(
                pool,
                task_id,
                TaskStatus::Completed,
                None,
                Some(completed_at),
                None,
                Some(&result),
            )
            .await
            {
                tracing::error!(%task_id, error = %e, "failed to mark task as completed");
            }

            // Broadcast completion
            let update = TaskProgress {
                task_id,
                task_type: progress.task_type(),
                status: TaskStatus::Completed,
                progress: 100,
                message: None,
                result: Some(result),
                error: None,
                parent_task_id,
                data: None,
            };
            let _ = progress.tx.send(update);

            tracing::info!(%task_id, "task completed");
        }
        Err(TaskError::Cancelled) => {
            let completed_at = Utc::now();
            if let Err(db_err) = TaskRepository::update_status(
                pool,
                task_id,
                TaskStatus::Cancelled,
                None,
                Some(completed_at),
                None,
                None,
            )
            .await
            {
                tracing::error!(%task_id, error = %db_err, "failed to mark task as cancelled");
            }

            // Cancel pending children
            if let Err(e) = TaskRepository::cancel_pending_children(pool, task_id).await {
                tracing::warn!(%task_id, error = %e, "failed to cancel pending children");
            }

            // Broadcast cancellation
            let update = TaskProgress {
                task_id,
                task_type: progress.task_type(),
                status: TaskStatus::Cancelled,
                progress: 0,
                message: Some("Task cancelled".into()),
                result: None,
                error: None,
                parent_task_id,
                data: None,
            };
            let _ = progress.tx.send(update);

            tracing::info!(%task_id, "task cancelled");
        }
        Err(e) => {
            let completed_at = Utc::now();
            let error_msg = e.to_string();
            if let Err(db_err) = TaskRepository::update_status(
                pool,
                task_id,
                TaskStatus::Failed,
                None,
                Some(completed_at),
                Some(&error_msg),
                None,
            )
            .await
            {
                tracing::error!(%task_id, error = %db_err, "failed to mark task as failed");
            }

            // Broadcast failure
            let update = TaskProgress {
                task_id,
                task_type: progress.task_type(),
                status: TaskStatus::Failed,
                progress: 0,
                message: None,
                result: None,
                error: Some(error_msg.clone()),
                parent_task_id,
                data: None,
            };
            let _ = progress.tx.send(update);

            tracing::warn!(%task_id, error = %error_msg, "task failed");
        }
    }
}

/// Recover tasks that were interrupted (running when the app stopped) and
/// re-enqueue all pending tasks.
pub async fn recover_tasks(
    pool: &DbPool,
    dispatch_tx: &mpsc::Sender<Task>,
) -> Result<usize, TaskError> {
    let repaired = repair_resolution_lifecycle(pool).await?;
    let tasks = TaskRepository::recover_interrupted(pool)
        .await
        .map_err(|e| TaskError::Internal(e.to_string()))?;

    let count = tasks.len();
    for task in tasks {
        dispatch_tx
            .send(task)
            .await
            .map_err(|_| TaskError::QueueFull)?;
    }

    if count > 0 {
        tracing::info!(count, "recovered pending tasks");
    }
    if repaired.runs > 0 || repaired.books > 0 {
        tracing::info!(
            recovered_runs = repaired.runs,
            reset_books = repaired.books,
            "repaired resolution lifecycle after restart"
        );
    }

    Ok(count)
}

struct ResolutionRecoverySummary {
    runs: usize,
    books: usize,
}

async fn repair_resolution_lifecycle(
    pool: &DbPool,
) -> Result<ResolutionRecoverySummary, TaskError> {
    let mut repaired_runs = 0;
    for mut run in ResolutionRunRepository::list_running(pool)
        .await
        .map_err(|e| TaskError::Internal(e.to_string()))?
    {
        let book = BookRepository::get_by_id(pool, run.book_id)
            .await
            .map_err(|e| TaskError::Internal(e.to_string()))?;

        if book.resolution_requested_at > run.started_at {
            CandidateRepository::mark_run_superseded(pool, run.id)
                .await
                .map_err(|e| TaskError::Internal(e.to_string()))?;
            run.state = ResolutionRunState::Superseded;
            run.decision_code = "superseded".into();
            run.error = None;
        } else {
            run.state = ResolutionRunState::Failed;
            run.decision_code = "failed".into();
            run.error = Some("interrupted by restart".into());
        }

        run.outcome = None;
        run.finished_at = Some(Utc::now());
        ResolutionRunRepository::finalize(pool, &run)
            .await
            .map_err(|e| TaskError::Internal(e.to_string()))?;
        repaired_runs += 1;
    }

    let mut reset_books = 0;
    for book in BookRepository::list_running_resolution(pool)
        .await
        .map_err(|e| TaskError::Internal(e.to_string()))?
    {
        if BookRepository::reset_resolution_to_pending(pool, book.id)
            .await
            .map_err(|e| TaskError::Internal(e.to_string()))?
        {
            reset_books += 1;
        }
    }

    Ok(ResolutionRecoverySummary {
        runs: repaired_runs,
        books: reset_books,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use archivis_core::models::{
        Book, CandidateStatus, IdentificationCandidate, ResolutionRun, ResolutionState, Task,
    };
    use chrono::Duration;

    async fn test_pool() -> (archivis_db::DbPool, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let pool = archivis_db::create_pool(&db_path).await.unwrap();
        archivis_db::run_migrations(&pool).await.unwrap();
        (pool, dir)
    }

    #[tokio::test]
    async fn recover_tasks_repairs_superseded_runs_and_requeues_running_tasks() {
        let (pool, _dir) = test_pool().await;
        let requested_at = Utc::now();

        let mut book = Book::new("Recovery Superseded");
        book.resolution_state = ResolutionState::Running;
        book.resolution_requested_at = requested_at;
        BookRepository::create(&pool, &book).await.unwrap();

        let mut run = ResolutionRun::new(book.id, "automatic", serde_json::json!({}));
        run.state = ResolutionRunState::Running;
        run.started_at = requested_at - Duration::minutes(5);
        ResolutionRunRepository::create(&pool, &run).await.unwrap();

        book.last_resolution_run_id = Some(run.id);
        BookRepository::update(&pool, &book).await.unwrap();

        let mut candidate =
            IdentificationCandidate::new(book.id, "provider", 0.91, serde_json::json!({}), vec![]);
        candidate.run_id = Some(run.id);
        CandidateRepository::create(&pool, &candidate)
            .await
            .unwrap();

        let task = Task::new(
            TaskType::ResolveBook,
            serde_json::json!({ "book_id": book.id.to_string() }),
        );
        TaskRepository::create(&pool, &task).await.unwrap();
        TaskRepository::update_status(
            &pool,
            task.id,
            TaskStatus::Running,
            Some(Utc::now()),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let (dispatch_tx, mut dispatch_rx) = mpsc::channel(4);
        let count = recover_tasks(&pool, &dispatch_tx).await.unwrap();

        assert_eq!(count, 1);
        let recovered_task = dispatch_rx.recv().await.unwrap();
        assert_eq!(recovered_task.id, task.id);
        assert_eq!(recovered_task.status, TaskStatus::Pending);

        let recovered_task_row = TaskRepository::get_by_id(&pool, task.id).await.unwrap();
        assert_eq!(recovered_task_row.status, TaskStatus::Pending);

        let recovered_run = ResolutionRunRepository::get_by_id(&pool, run.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered_run.state, ResolutionRunState::Superseded);

        let recovered_candidate = CandidateRepository::get_by_id(&pool, candidate.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered_candidate.status, CandidateStatus::Superseded);

        let recovered_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
        assert_eq!(recovered_book.resolution_state, ResolutionState::Pending);
    }

    #[tokio::test]
    async fn recover_tasks_marks_interrupted_runs_failed_when_not_superseded() {
        let (pool, _dir) = test_pool().await;
        let requested_at = Utc::now();

        let mut book = Book::new("Recovery Failed");
        book.resolution_state = ResolutionState::Running;
        book.resolution_requested_at = requested_at;
        BookRepository::create(&pool, &book).await.unwrap();

        let mut run = ResolutionRun::new(book.id, "automatic", serde_json::json!({}));
        run.state = ResolutionRunState::Running;
        run.started_at = requested_at;
        ResolutionRunRepository::create(&pool, &run).await.unwrap();

        book.last_resolution_run_id = Some(run.id);
        BookRepository::update(&pool, &book).await.unwrap();

        let (dispatch_tx, _dispatch_rx) = mpsc::channel(1);
        recover_tasks(&pool, &dispatch_tx).await.unwrap();

        let recovered_run = ResolutionRunRepository::get_by_id(&pool, run.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered_run.state, ResolutionRunState::Failed);
        assert_eq!(
            recovered_run.error.as_deref(),
            Some("interrupted by restart")
        );

        let recovered_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
        assert_eq!(recovered_book.resolution_state, ResolutionState::Pending);
    }
}
