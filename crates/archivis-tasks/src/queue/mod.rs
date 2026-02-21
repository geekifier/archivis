mod worker;

pub use worker::{ProgressSender, Worker};

use std::collections::HashMap;
use std::sync::Arc;

use archivis_core::errors::TaskError;
use archivis_core::models::{Task, TaskProgress, TaskStatus, TaskType};
use archivis_db::{DbPool, TaskRepository};
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
) {
    tracing::info!(worker_count = workers.len(), "task dispatcher started",);

    while let Some(task) = rx.recv().await {
        let task_id = task.id;
        let task_type = task.task_type;

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
        let progress = progress.for_task(task_id);
        let pool = db_pool.clone();

        tokio::spawn(async move {
            dispatch_task(task_id, task.payload, &*worker, progress, &pool).await;
        });
    }

    tracing::info!("task dispatcher stopped");
}

/// Execute a single task: mark running, call worker, mark completed/failed.
async fn dispatch_task(
    task_id: uuid::Uuid,
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
                status: TaskStatus::Completed,
                progress: 100,
                message: None,
                result: Some(result),
                error: None,
            };
            let _ = progress.tx.send(update);

            tracing::info!(%task_id, "task completed");
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
                status: TaskStatus::Failed,
                progress: 0,
                message: None,
                result: None,
                error: Some(error_msg.clone()),
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

    Ok(count)
}
