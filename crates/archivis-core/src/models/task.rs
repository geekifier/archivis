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
}

impl fmt::Display for TaskType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ImportFile => write!(f, "import_file"),
            Self::ImportDirectory => write!(f, "import_directory"),
        }
    }
}

impl FromStr for TaskType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "import_file" => Ok(Self::ImportFile),
            "import_directory" => Ok(Self::ImportDirectory),
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
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
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
            other => Err(format!("unknown task status: {other}")),
        }
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
        }
    }
}

/// Progress update broadcast to SSE subscribers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskProgress {
    pub task_id: Uuid,
    pub status: TaskStatus,
    pub progress: u8,
    pub message: Option<String>,
    /// Present only on completion.
    pub result: Option<serde_json::Value>,
    /// Present only on failure.
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_type_display_and_parse() {
        assert_eq!(TaskType::ImportFile.to_string(), "import_file");
        assert_eq!(TaskType::ImportDirectory.to_string(), "import_directory");
        assert_eq!(
            "import_file".parse::<TaskType>().unwrap(),
            TaskType::ImportFile,
        );
        assert_eq!(
            "import_directory".parse::<TaskType>().unwrap(),
            TaskType::ImportDirectory,
        );
        assert!("bogus".parse::<TaskType>().is_err());
    }

    #[test]
    fn task_type_serde_roundtrip() {
        let tt = TaskType::ImportFile;
        let json = serde_json::to_string(&tt).unwrap();
        assert_eq!(json, r#""import_file""#);
        let deserialized: TaskType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, tt);
    }

    #[test]
    fn task_status_display_and_parse() {
        assert_eq!(TaskStatus::Pending.to_string(), "pending");
        assert_eq!(TaskStatus::Running.to_string(), "running");
        assert_eq!(TaskStatus::Completed.to_string(), "completed");
        assert_eq!(TaskStatus::Failed.to_string(), "failed");
        assert_eq!(
            "pending".parse::<TaskStatus>().unwrap(),
            TaskStatus::Pending,
        );
        assert_eq!(
            "running".parse::<TaskStatus>().unwrap(),
            TaskStatus::Running,
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
    }
}
