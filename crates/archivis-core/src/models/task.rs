use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The kind of background task to execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    ImportFile,
    ImportDirectory,
    ResolveBook,
    ScanIsbn,
}

impl fmt::Display for TaskType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ImportFile => write!(f, "import_file"),
            Self::ImportDirectory => write!(f, "import_directory"),
            Self::ResolveBook => write!(f, "resolve_book"),
            Self::ScanIsbn => write!(f, "scan_isbn"),
        }
    }
}

impl FromStr for TaskType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "import_file" => Ok(Self::ImportFile),
            "import_directory" => Ok(Self::ImportDirectory),
            "resolve_book" => Ok(Self::ResolveBook),
            "scan_isbn" => Ok(Self::ScanIsbn),
            other => Err(format!("unknown task type: {other}")),
        }
    }
}

/// Lifecycle status of a background task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl FromStr for TaskStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(format!("unknown task status: {other}")),
        }
    }
}

impl TaskStatus {
    /// Returns `true` if the task is in a terminal state (completed, failed, or cancelled).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// A persisted background task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub task_type: TaskType,
    pub payload: serde_json::Value,
    pub status: TaskStatus,
    pub progress: u8,
    pub message: Option<String>,
    pub result: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub parent_task_id: Option<Uuid>,
}

impl Task {
    /// Create a new pending task with the given type and payload.
    pub fn new(task_type: TaskType, payload: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            task_type,
            payload,
            status: TaskStatus::Pending,
            progress: 0,
            message: None,
            result: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            error_message: None,
            parent_task_id: None,
        }
    }

    /// Create a new pending child task linked to a parent.
    pub fn new_child(task_type: TaskType, payload: serde_json::Value, parent_id: Uuid) -> Self {
        let mut task = Self::new(task_type, payload);
        task.parent_task_id = Some(parent_id);
        task
    }
}

/// Progress update broadcast to SSE subscribers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskProgress {
    pub task_id: Uuid,
    pub task_type: TaskType,
    pub status: TaskStatus,
    pub progress: u8,
    pub message: Option<String>,
    /// Present only on completion.
    pub result: Option<serde_json::Value>,
    /// Present only on failure.
    pub error: Option<String>,
    /// Parent task ID for hierarchy grouping.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<Uuid>,
    /// Structured progress data (e.g. import counters).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_type_display_and_parse() {
        assert_eq!(TaskType::ImportFile.to_string(), "import_file");
        assert_eq!(TaskType::ImportDirectory.to_string(), "import_directory");
        assert_eq!(TaskType::ResolveBook.to_string(), "resolve_book");
        assert_eq!(TaskType::ScanIsbn.to_string(), "scan_isbn");
        assert_eq!(
            "import_file".parse::<TaskType>().unwrap(),
            TaskType::ImportFile,
        );
        assert_eq!(
            "import_directory".parse::<TaskType>().unwrap(),
            TaskType::ImportDirectory,
        );
        assert_eq!(
            "resolve_book".parse::<TaskType>().unwrap(),
            TaskType::ResolveBook,
        );
        assert_eq!("scan_isbn".parse::<TaskType>().unwrap(), TaskType::ScanIsbn,);
        assert!("bogus".parse::<TaskType>().is_err());
    }

    #[test]
    fn task_type_serde_roundtrip() {
        let tt = TaskType::ImportFile;
        let json = serde_json::to_string(&tt).unwrap();
        assert_eq!(json, r#""import_file""#);
        let deserialized: TaskType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, tt);

        let tt2 = TaskType::ResolveBook;
        let json2 = serde_json::to_string(&tt2).unwrap();
        assert_eq!(json2, r#""resolve_book""#);
        let deserialized2: TaskType = serde_json::from_str(&json2).unwrap();
        assert_eq!(deserialized2, tt2);

        let tt3 = TaskType::ScanIsbn;
        let json3 = serde_json::to_string(&tt3).unwrap();
        assert_eq!(json3, r#""scan_isbn""#);
        let deserialized3: TaskType = serde_json::from_str(&json3).unwrap();
        assert_eq!(deserialized3, tt3);
    }

    #[test]
    fn task_status_display_and_parse() {
        assert_eq!(TaskStatus::Pending.to_string(), "pending");
        assert_eq!(TaskStatus::Running.to_string(), "running");
        assert_eq!(TaskStatus::Completed.to_string(), "completed");
        assert_eq!(TaskStatus::Failed.to_string(), "failed");
        assert_eq!(TaskStatus::Cancelled.to_string(), "cancelled");
        assert_eq!(
            "pending".parse::<TaskStatus>().unwrap(),
            TaskStatus::Pending,
        );
        assert_eq!(
            "running".parse::<TaskStatus>().unwrap(),
            TaskStatus::Running,
        );
        assert_eq!(
            "cancelled".parse::<TaskStatus>().unwrap(),
            TaskStatus::Cancelled,
        );
        assert!("unknown".parse::<TaskStatus>().is_err());
    }

    #[test]
    fn task_status_serde_roundtrip() {
        let ts = TaskStatus::Completed;
        let json = serde_json::to_string(&ts).unwrap();
        assert_eq!(json, r#""completed""#);
        let deserialized: TaskStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ts);

        let ts2 = TaskStatus::Cancelled;
        let json2 = serde_json::to_string(&ts2).unwrap();
        assert_eq!(json2, r#""cancelled""#);
        let deserialized2: TaskStatus = serde_json::from_str(&json2).unwrap();
        assert_eq!(deserialized2, ts2);
    }

    #[test]
    fn task_status_is_terminal() {
        assert!(!TaskStatus::Pending.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Cancelled.is_terminal());
    }

    #[test]
    fn new_task_has_defaults() {
        let task = Task::new(
            TaskType::ImportFile,
            serde_json::json!({"path": "/tmp/book.epub"}),
        );
        assert_eq!(task.task_type, TaskType::ImportFile);
        assert_eq!(task.status, TaskStatus::Pending);
        assert_eq!(task.progress, 0);
        assert!(task.message.is_none());
        assert!(task.result.is_none());
        assert!(task.started_at.is_none());
        assert!(task.completed_at.is_none());
        assert!(task.error_message.is_none());
        assert!(task.parent_task_id.is_none());
    }

    #[test]
    fn new_child_task_has_parent() {
        let parent_id = Uuid::new_v4();
        let task = Task::new_child(
            TaskType::ScanIsbn,
            serde_json::json!({"book_id": "abc"}),
            parent_id,
        );
        assert_eq!(task.parent_task_id, Some(parent_id));
        assert_eq!(task.task_type, TaskType::ScanIsbn);
    }
}
