use std::future::Future;
use std::pin::Pin;

use archivis_core::errors::TaskError;
use archivis_core::models::{TaskProgress, TaskStatus, TaskType};
use archivis_db::{DbPool, TaskRepository};
use tokio::sync::broadcast;
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
    pub(crate) tx: broadcast::Sender<TaskProgress>,
    db_pool: DbPool,
}

impl ProgressSender {
    pub fn new(tx: broadcast::Sender<TaskProgress>, db_pool: DbPool) -> Self {
        Self {
            task_id: Uuid::nil(),
            tx,
            db_pool,
        }
    }

    /// Create a copy scoped to a specific task.
    #[must_use]
    pub fn for_task(&self, task_id: Uuid) -> Self {
        Self {
            task_id,
            tx: self.tx.clone(),
            db_pool: self.db_pool.clone(),
        }
    }

    /// Report progress (0-100) with an optional message.
    pub async fn send_progress(&self, progress: u8, message: Option<String>) {
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
            status: TaskStatus::Running,
            progress,
            message,
            result: None,
            error: None,
        };

        // Ignore send errors — no subscribers is fine.
        let _ = self.tx.send(update);
    }
}
