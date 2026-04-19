use std::path::{Path, PathBuf};

use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use axum::Json;
use validator::Validate;

use archivis_core::models::TaskType;
use archivis_tasks::import::{BulkImportService, ImportConfig, ImportService};

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    FormatSummary, ScanDirectoryRequest, ScanManifestResponse, StartImportRequest,
    TaskCreatedResponse, UploadResponse,
};

/// POST /api/import/upload -- upload one or more ebook files for import.
///
/// Accepts `multipart/form-data` with one or more file fields. Each file is
/// saved to a staging directory and an `ImportFile` background task is enqueued.
#[utoipa::path(
    post,
    path = "/api/import/upload",
    tag = "import",
    responses(
        (status = 202, description = "Files accepted, import tasks enqueued", body = UploadResponse),
        (status = 400, description = "No files provided or invalid upload"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn upload_files(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<UploadResponse>), ApiError> {
    let upload_dir = state.config().data_dir.join("uploads");
    tokio::fs::create_dir_all(&upload_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to create upload directory: {e}")))?;

    // Each upload batch gets its own subdirectory to avoid filename collisions.
    let batch_id = uuid::Uuid::new_v4();
    let batch_dir = upload_dir.join(batch_id.to_string());
    tokio::fs::create_dir_all(&batch_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to create batch directory: {e}")))?;

    let mut tasks = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::Validation(format!("multipart error: {e}")))?
    {
        let original_name = field.file_name().map_or_else(
            || format!("{}.bin", uuid::Uuid::new_v4()),
            ToString::to_string,
        );

        let safe_name = sanitize_filename(&original_name);

        let data = field
            .bytes()
            .await
            .map_err(|e| ApiError::Validation(format!("failed to read upload: {e}")))?;

        if data.is_empty() {
            continue;
        }

        let file_path = batch_dir.join(&safe_name);
        tokio::fs::write(&file_path, &data)
            .await
            .map_err(|e| ApiError::Internal(format!("failed to write uploaded file: {e}")))?;

        let payload = serde_json::json!({
            "file_path": file_path.to_string_lossy(),
        });

        let task_id = state
            .task_queue()
            .enqueue(TaskType::ImportFile, payload)
            .await
            .map_err(|e| ApiError::Core(archivis_core::errors::ArchivisError::from(e)))?;

        tasks.push(TaskCreatedResponse { task_id });
    }

    if tasks.is_empty() {
        // Clean up the empty batch directory.
        let _ = tokio::fs::remove_dir(&batch_dir).await;
        return Err(ApiError::Validation("no files provided".into()));
    }

    Ok((StatusCode::ACCEPTED, Json(UploadResponse { tasks })))
}

/// POST /api/import/scan -- scan a directory for importable ebook files.
///
/// Returns a manifest with file counts, detected formats, and total size.
/// This is a fast read-only operation that does not import anything.
#[utoipa::path(
    post,
    path = "/api/import/scan",
    tag = "import",
    request_body = ScanDirectoryRequest,
    responses(
        (status = 200, description = "Directory scan manifest", body = ScanManifestResponse),
        (status = 400, description = "Invalid or inaccessible directory path"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn scan_directory(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<ScanDirectoryRequest>,
) -> Result<Json<ScanManifestResponse>, ApiError> {
    body.validate()?;

    let dir_path = PathBuf::from(&body.path);
    validate_directory_path(&dir_path).await?;

    let import_config = ImportConfig {
        data_dir: state.config().data_dir.clone(),
        ..ImportConfig::default()
    };
    let import_service = ImportService::new(
        state.db_pool().clone(),
        state.storage().clone(),
        import_config,
        state.settings_reader(),
    );
    let bulk_service = BulkImportService::new(import_service);

    let manifest = bulk_service
        .scan_directory(&dir_path)
        .await
        .map_err(map_import_error)?;

    let mut formats: Vec<FormatSummary> = manifest
        .by_format
        .into_iter()
        .map(|(format, count)| FormatSummary {
            format: format.to_string(),
            count: count.count,
            total_size: count.total_size,
        })
        .collect();

    // Sort by count descending for a predictable, useful ordering.
    formats.sort_by_key(|f| std::cmp::Reverse(f.count));

    Ok(Json(ScanManifestResponse {
        total_files: manifest.total_files,
        total_size: manifest.total_size,
        formats,
    }))
}

/// POST /api/import/scan/start -- start bulk import from a directory.
///
/// Enqueues an `ImportDirectory` background task for the given directory.
/// Use the returned task ID to track progress via SSE.
#[utoipa::path(
    post,
    path = "/api/import/scan/start",
    tag = "import",
    request_body = StartImportRequest,
    responses(
        (status = 202, description = "Import task enqueued", body = TaskCreatedResponse),
        (status = 400, description = "Invalid or inaccessible directory path"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn start_import(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<StartImportRequest>,
) -> Result<(StatusCode, Json<TaskCreatedResponse>), ApiError> {
    body.validate()?;

    let dir_path = PathBuf::from(&body.path);
    validate_directory_path(&dir_path).await?;

    let payload = serde_json::json!({
        "directory_path": body.path,
    });

    let task_id = state
        .task_queue()
        .enqueue(TaskType::ImportDirectory, payload)
        .await
        .map_err(|e| ApiError::Core(archivis_core::errors::ArchivisError::from(e)))?;

    Ok((StatusCode::ACCEPTED, Json(TaskCreatedResponse { task_id })))
}

/// Validate that a path is an existing, absolute directory.
async fn validate_directory_path(path: &Path) -> Result<(), ApiError> {
    if !path.is_absolute() {
        return Err(ApiError::Validation("path must be absolute".into()));
    }

    let metadata = tokio::fs::metadata(path).await.map_err(|e| {
        ApiError::Validation(format!("cannot access path '{}': {e}", path.display()))
    })?;

    if !metadata.is_dir() {
        return Err(ApiError::Validation(format!(
            "path is not a directory: {}",
            path.display()
        )));
    }

    Ok(())
}

/// Map an `ImportError` to an appropriate `ApiError`.
fn map_import_error(err: archivis_tasks::import::ImportError) -> ApiError {
    use archivis_tasks::import::ImportError;

    match err {
        ImportError::InvalidFile(msg) => ApiError::Validation(msg),
        ImportError::Io(io_err) => {
            ApiError::Validation(format!("cannot access directory: {io_err}"))
        }
        ImportError::FormatDetection(e) => {
            ApiError::Core(archivis_core::errors::ArchivisError::Format(e))
        }
        ImportError::Storage(e) => ApiError::Core(archivis_core::errors::ArchivisError::Storage(e)),
        ImportError::Database(e) => ApiError::Core(archivis_core::errors::ArchivisError::Db(e)),
        ImportError::DuplicateFile { .. } => ApiError::Internal(err.to_string()),
    }
}

/// Sanitize a filename to prevent path traversal and other issues.
///
/// Strips directory components, replaces dangerous characters, and falls
/// back to a UUID-based name if the result is empty.
fn sanitize_filename(name: &str) -> String {
    // Extract just the filename component (strip any directory prefix).
    let name = Path::new(name)
        .file_name()
        .map_or_else(|| name.to_string(), |n| n.to_string_lossy().into_owned());

    if name.is_empty() || name == "." || name == ".." {
        return format!("{}.bin", uuid::Uuid::new_v4());
    }

    // Replace path separators and null bytes.
    name.replace(['/', '\\', '\0'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_directory_prefix() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("/absolute/path/book.epub"), "book.epub");
    }

    #[test]
    fn sanitize_handles_empty_and_dots() {
        let result = sanitize_filename("");
        assert!(
            Path::new(&result)
                .extension()
                .is_some_and(|ext| ext == "bin"),
            "expected UUID.bin, got: {result}"
        );

        let result = sanitize_filename("..");
        assert!(
            Path::new(&result)
                .extension()
                .is_some_and(|ext| ext == "bin"),
            "expected UUID.bin, got: {result}"
        );
    }

    #[test]
    fn sanitize_replaces_dangerous_chars() {
        assert_eq!(sanitize_filename("file\0name.epub"), "file_name.epub");
        assert_eq!(sanitize_filename("sub/dir.epub"), "dir.epub");
    }

    #[test]
    fn sanitize_preserves_normal_filename() {
        assert_eq!(
            sanitize_filename("Dune - Frank Herbert.epub"),
            "Dune - Frank Herbert.epub"
        );
    }
}
