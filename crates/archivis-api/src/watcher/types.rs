use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Request body for adding a new watched directory.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddWatchedDirectoryRequest {
    /// Absolute path to the directory to watch.
    pub path: String,
    /// Watch backend: `"native"` or `"poll"`. Defaults to `"poll"`.
    pub watch_mode: Option<String>,
    /// Per-directory polling interval override (seconds). Null = use global default.
    pub poll_interval_secs: Option<i64>,
}

/// Request body for updating an existing watched directory.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateWatchedDirectoryRequest {
    /// Watch backend: `"native"` or `"poll"`.
    pub watch_mode: Option<String>,
    /// Per-directory polling interval override (seconds).
    /// `None` = don't change, `Some(None)` = clear override (use global default).
    pub poll_interval_secs: Option<Option<i64>>,
    /// Enable or disable the watched directory.
    pub enabled: Option<bool>,
}

/// Response representing a single watched directory.
#[derive(Debug, Serialize, ToSchema)]
pub struct WatchedDirectoryResponse {
    /// Unique identifier.
    pub id: String,
    /// Absolute path being watched.
    pub path: String,
    /// Watch backend: `"native"` or `"poll"` -- the user's explicit choice.
    pub watch_mode: String,
    /// Per-directory polling interval override (seconds), if set.
    pub poll_interval_secs: Option<i64>,
    /// Resolved polling interval: per-directory override or global default.
    pub effective_poll_interval_secs: i64,
    /// Whether this directory is actively being watched.
    pub enabled: bool,
    /// Most recent watcher error, if any.
    pub last_error: Option<String>,
    /// Filesystem detection hint (populated on GET and POST).
    pub detected_fs: Option<FsDetectionResponse>,
    /// ISO 8601 timestamp of when this directory was added.
    pub created_at: String,
    /// ISO 8601 timestamp of last update.
    pub updated_at: String,
}

/// Filesystem detection hint returned alongside a watched directory.
#[derive(Debug, Serialize, ToSchema)]
pub struct FsDetectionResponse {
    /// Detected filesystem type (e.g., "ext4", "NFS", "CIFS", "FUSE", "unknown").
    pub fs_type: String,
    /// Whether native OS events are expected to work: "likely", "unlikely", or "unknown".
    pub native_likely_works: String,
    /// User-facing explanation of the detection result.
    pub explanation: String,
}

/// Request body for the filesystem detection endpoint.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DetectFsRequest {
    /// Absolute path to detect filesystem type for.
    pub path: String,
}

/// Response from triggering a manual scan of a watched directory.
#[derive(Debug, Serialize, ToSchema)]
pub struct ScanTriggeredResponse {
    /// ID of the created background import task.
    pub task_id: Uuid,
}
