use std::future::Future;
use std::pin::Pin;

use archivis_core::errors::TaskError;
use archivis_core::models::{TaskProgress, TaskStatus, TaskType};
use archivis_db::{DbPool, TaskRepository};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// A worker that processes tasks of a specific type.
///
/// Uses boxed futures for dyn-safety without requiring the `async-trait` crate.
pub trait Worker: Send + Sync {
    /// The task type this worker handles.
    fn task_type(&self) -> TaskType;

    /// Execute the task with the given payload, reporting progress through the sender.
    fn execute(
        &self,
        payload: serde_json::Value,
        progress: ProgressSender,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, TaskError>> + Send + '_>>;
}

/// Sends progress updates for a specific task to both the database and broadcast subscribers.
#[derive(Clone)]
pub struct ProgressSender {
    task_id: Uuid,
    task_type: TaskType,
    parent_task_id: Option<Uuid>,
    pub(crate) tx: broadcast::Sender<TaskProgress>,
    db_pool: DbPool,
    cancellation_token: Option<CancellationToken>,
}

impl ProgressSender {
    pub fn new(tx: broadcast::Sender<TaskProgress>, db_pool: DbPool) -> Self {
        Self {
            task_id: Uuid::nil(),
            task_type: TaskType::ImportFile,
            parent_task_id: None,
            tx,
            db_pool,
            cancellation_token: None,
        }
    }

    /// Create a copy scoped to a specific task.
    #[must_use]
    pub fn for_task(&self, task_id: Uuid) -> Self {
        Self {
            task_id,
            task_type: TaskType::ImportFile,
            parent_task_id: None,
            tx: self.tx.clone(),
            db_pool: self.db_pool.clone(),
            cancellation_token: None,
        }
    }

    /// Set the parent task ID for hierarchy grouping in SSE events.
    #[must_use]
    pub fn with_parent(mut self, parent_task_id: Option<Uuid>) -> Self {
        self.parent_task_id = parent_task_id;
        self
    }

    /// Set the task type for SSE events.
    #[must_use]
    pub fn with_task_type(mut self, task_type: TaskType) -> Self {
        self.task_type = task_type;
        self
    }

    /// Set the cancellation token.
    #[must_use]
    pub fn with_cancellation_token(mut self, token: CancellationToken) -> Self {
        self.cancellation_token = Some(token);
        self
    }

    /// Check if this task has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancellation_token
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }

    /// Returns a future that resolves when cancellation is requested.
    /// Useful for `tokio::select!`.
    pub async fn cancelled(&self) {
        match &self.cancellation_token {
            Some(token) => token.cancelled().await,
            None => std::future::pending().await,
        }
    }

    /// Get the task ID.
    pub fn task_id(&self) -> Uuid {
        self.task_id
    }

    /// Get the parent task ID (if this task is a child).
    pub fn parent_task_id(&self) -> Option<Uuid> {
        self.parent_task_id
    }

    /// Get the parent task ID, falling back to this task's own ID.
    /// Used when enqueueing child tasks that should resolve to the top-level parent.
    pub fn resolution_parent(&self) -> Uuid {
        self.parent_task_id.unwrap_or(self.task_id)
    }

    /// Get the task type.
    pub fn task_type(&self) -> TaskType {
        self.task_type
    }

    /// Get the broadcast sender (for direct sends in dispatch).
    pub fn broadcast_sender(&self) -> &broadcast::Sender<TaskProgress> {
        &self.tx
    }

    /// Report progress (0-100) with an optional message.
    pub async fn send_progress(&self, progress: u8, message: Option<String>) {
        self.send_progress_with_data(progress, message, None).await;
    }

    /// Report progress (0-100) with an optional message and structured data.
    pub async fn send_progress_with_data(
        &self,
        progress: u8,
        message: Option<String>,
        data: Option<serde_json::Value>,
    ) {
        // Best-effort: update DB then broadcast. Errors are logged but not propagated
        // since failing to report progress should not abort the task.
        if let Err(e) = TaskRepository::update_progress(
            &self.db_pool,
            self.task_id,
            progress,
            message.as_deref(),
        )
        .await
        {
            tracing::warn!(task_id = %self.task_id, error = %e, "failed to persist progress update");
        }

        let update = TaskProgress {
            task_id: self.task_id,
            task_type: self.task_type,
            status: TaskStatus::Running,
            progress,
            message,
            result: None,
            error: None,
            parent_task_id: self.parent_task_id,
            data,
        };

        // Ignore send errors — no subscribers is fine.
        let _ = self.tx.send(update);
    }

    /// Get a reference to the DB pool.
    pub fn db_pool(&self) -> &DbPool {
        &self.db_pool
    }

    /// Get the Arc<CancellationToken> if set.
    pub fn cancellation_token(&self) -> Option<&CancellationToken> {
        self.cancellation_token.as_ref()
    }
}
