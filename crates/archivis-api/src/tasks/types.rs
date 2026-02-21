use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use archivis_core::models::{TaskStatus, TaskType};

/// JSON response for a single task.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskResponse {
    pub id: Uuid,
    #[schema(value_type = String, example = "ImportFile")]
    pub task_type: TaskType,
    #[schema(value_type = String, example = "Running")]
    pub status: TaskStatus,
    pub progress: u8,
    pub message: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub result: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
}

impl From<archivis_core::models::Task> for TaskResponse {
    fn from(task: archivis_core::models::Task) -> Self {
        Self {
            id: task.id,
            task_type: task.task_type,
            status: task.status,
            progress: task.progress,
            message: task.message,
            result: task.result,
            created_at: task.created_at,
            started_at: task.started_at,
            completed_at: task.completed_at,
            error_message: task.error_message,
        }
    }
}
