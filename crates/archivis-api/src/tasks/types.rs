use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use archivis_core::models::{TaskStatus, TaskType};
use archivis_db::ChildTaskSummary;

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
    pub parent_task_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children_summary: Option<ChildrenSummaryResponse>,
}

/// Summary of child task statuses.
#[derive(Debug, Serialize, ToSchema)]
pub struct ChildrenSummaryResponse {
    pub total: i64,
    pub pending: i64,
    pub running: i64,
    pub completed: i64,
    pub failed: i64,
    pub cancelled: i64,
}

impl From<ChildTaskSummary> for ChildrenSummaryResponse {
    fn from(s: ChildTaskSummary) -> Self {
        Self {
            total: s.total,
            pending: s.pending,
            running: s.running,
            completed: s.completed,
            failed: s.failed,
            cancelled: s.cancelled,
        }
    }
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
            parent_task_id: task.parent_task_id,
            children_summary: None,
        }
    }
}
