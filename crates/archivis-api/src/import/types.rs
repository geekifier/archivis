use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

/// Request body for scanning a directory.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ScanDirectoryRequest {
    /// Absolute path to the directory to scan.
    #[validate(length(min = 1, message = "path must not be empty"))]
    pub path: String,
}

/// Request body for starting a bulk import from a scanned directory.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct StartImportRequest {
    /// Absolute path to the directory to import.
    #[validate(length(min = 1, message = "path must not be empty"))]
    pub path: String,
}

/// Per-format file count and size in a scan manifest.
#[derive(Debug, Serialize, ToSchema)]
pub struct FormatSummary {
    /// Format name (e.g. "EPUB", "PDF").
    pub format: String,
    /// Number of files in this format.
    pub count: usize,
    /// Total size of files in this format (bytes).
    pub total_size: u64,
}

/// Response from scanning a directory for importable ebook files.
#[derive(Debug, Serialize, ToSchema)]
pub struct ScanManifestResponse {
    /// Total number of importable files found.
    pub total_files: usize,
    /// Total size of all importable files (bytes).
    pub total_size: u64,
    /// Breakdown by detected format.
    pub formats: Vec<FormatSummary>,
}

/// Response containing a single background task reference.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskCreatedResponse {
    /// ID of the created background task (use for progress tracking).
    pub task_id: Uuid,
}

/// Response from uploading one or more ebook files.
#[derive(Debug, Serialize, ToSchema)]
pub struct UploadResponse {
    /// One task per uploaded file.
    pub tasks: Vec<TaskCreatedResponse>,
}
