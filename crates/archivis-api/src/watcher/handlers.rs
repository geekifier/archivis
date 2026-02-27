use std::path::{Path, PathBuf};

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use archivis_core::models::{TaskType, WatchMode};
use archivis_db::WatchedDirectoryRepository;
use archivis_storage::watcher::{detect_fs_type, FsDetectionResult};

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    AddWatchedDirectoryRequest, DetectFsRequest, FsDetectionResponse, ScanTriggeredResponse,
    UpdateWatchedDirectoryRequest, WatchedDirectoryResponse,
};

/// Default polling interval in seconds when no per-directory or global override is set.
const DEFAULT_POLL_INTERVAL_SECS: i64 = 30;

// ── Path validation helpers ─────────────────────────────────────────────

/// Validate that a path is absolute, exists, and is a directory.
/// Returns the canonicalized path on success.
async fn validate_directory_path(raw_path: &str) -> Result<PathBuf, ApiError> {
    let user_path = PathBuf::from(raw_path);
    if !user_path.is_absolute() {
        return Err(ApiError::Validation("path must be absolute".into()));
    }

    let canonical = tokio::fs::canonicalize(&user_path).await.map_err(|e| {
        ApiError::Validation(format!("cannot access path '{}': {e}", user_path.display()))
    })?;

    let meta = tokio::fs::metadata(&canonical).await.map_err(|e| {
        ApiError::Validation(format!("cannot access path '{}': {e}", canonical.display()))
    })?;
    if !meta.is_dir() {
        return Err(ApiError::Validation(format!(
            "path is not a directory: {}",
            canonical.display()
        )));
    }

    Ok(canonical)
}

/// Check that the given path is not inside the managed book storage directory.
fn validate_not_inside_storage(path: &Path, storage_root: &Path) -> Result<(), ApiError> {
    if path.starts_with(storage_root) {
        return Err(ApiError::Validation(
            "cannot watch a path inside the managed book storage directory".into(),
        ));
    }
    Ok(())
}

/// Return 503 when the watcher subsystem is disabled.
fn require_watcher_enabled(state: &AppState) -> Result<(), ApiError> {
    if state.watcher_service().is_none() {
        return Err(ApiError::ServiceUnavailable(
            "filesystem watcher is disabled -- set watcher.enabled = true in \
             configuration and restart the server"
                .into(),
        ));
    }
    Ok(())
}

/// Convert a `FsDetectionResult` into the API response type.
fn to_fs_detection_response(result: &FsDetectionResult) -> FsDetectionResponse {
    FsDetectionResponse {
        fs_type: result.fs_type.clone(),
        native_likely_works: result.native_likely_works.to_string(),
        explanation: result.explanation.clone(),
    }
}

/// Build a `WatchedDirectoryResponse` from a domain model, optionally including
/// live filesystem detection.
fn to_response(
    dir: &archivis_core::models::WatchedDirectory,
    detected_fs: Option<FsDetectionResponse>,
) -> WatchedDirectoryResponse {
    let effective_poll = dir.poll_interval_secs.unwrap_or(DEFAULT_POLL_INTERVAL_SECS);
    WatchedDirectoryResponse {
        id: dir.id.to_string(),
        path: dir.path.clone(),
        watch_mode: dir.watch_mode.to_string().to_lowercase(),
        poll_interval_secs: dir.poll_interval_secs,
        effective_poll_interval_secs: effective_poll,
        enabled: dir.enabled,
        last_error: dir.last_error.clone(),
        detected_fs,
        created_at: dir.created_at.to_rfc3339(),
        updated_at: dir.updated_at.to_rfc3339(),
    }
}

// ── Handlers ────────────────────────────────────────────────────────────

/// GET /api/watched-directories -- list all watched directories.
///
/// Returns both enabled and disabled directories. Includes live filesystem
/// detection hints for each directory.
#[utoipa::path(
    get,
    path = "/api/watched-directories",
    tag = "watched-directories",
    responses(
        (status = 200, description = "List of watched directories", body = Vec<WatchedDirectoryResponse>),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_watched(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
) -> Result<Json<Vec<WatchedDirectoryResponse>>, ApiError> {
    let dirs = WatchedDirectoryRepository::list_all(state.db_pool()).await?;

    let responses: Vec<WatchedDirectoryResponse> = dirs
        .iter()
        .map(|d| {
            let detection = detect_fs_type(Path::new(&d.path));
            to_response(d, Some(to_fs_detection_response(&detection)))
        })
        .collect();

    Ok(Json(responses))
}

/// POST /api/watched-directories -- add a new watched directory.
///
/// Validates the path, creates a database record, and starts watching
/// if the watcher service is enabled.
#[utoipa::path(
    post,
    path = "/api/watched-directories",
    tag = "watched-directories",
    request_body = AddWatchedDirectoryRequest,
    responses(
        (status = 201, description = "Watched directory created", body = WatchedDirectoryResponse),
        (status = 400, description = "Validation error (path invalid, inside storage)"),
        (status = 409, description = "Path already watched"),
        (status = 401, description = "Not authenticated"),
        (status = 503, description = "Watcher subsystem disabled"),
    ),
    security(("bearer" = []))
)]
pub async fn add_watched(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<AddWatchedDirectoryRequest>,
) -> Result<(StatusCode, Json<WatchedDirectoryResponse>), ApiError> {
    require_watcher_enabled(&state)?;

    let canonical = validate_directory_path(&body.path).await?;
    let canonical_str = canonical.to_string_lossy().to_string();

    // Check path is not inside managed storage.
    validate_not_inside_storage(&canonical, state.storage().root())?;

    // Check for duplicate path.
    if WatchedDirectoryRepository::exists_by_path(state.db_pool(), &canonical_str).await? {
        return Err(ApiError::Core(archivis_core::errors::ArchivisError::Db(
            archivis_core::errors::DbError::Constraint(format!(
                "path is already watched: {canonical_str}"
            )),
        )));
    }

    // Parse watch mode (default: poll).
    let watch_mode: WatchMode = body
        .watch_mode
        .as_deref()
        .unwrap_or("poll")
        .parse()
        .map_err(|e: String| ApiError::Validation(e))?;

    let dir = WatchedDirectoryRepository::create(
        state.db_pool(),
        &canonical_str,
        watch_mode,
        body.poll_interval_secs,
    )
    .await?;

    // Start watching if the watcher service is enabled.
    if let Some(ws) = state.watcher_service() {
        if let Err(e) = ws.read().await.watch(&dir).await {
            tracing::warn!(path = %dir.path, error = %e, "failed to start watching newly added directory");
        }
    }

    // Run FS detection for the response.
    let detection = detect_fs_type(&canonical);
    let response = to_response(&dir, Some(to_fs_detection_response(&detection)));

    Ok((StatusCode::CREATED, Json(response)))
}

/// GET /api/watched-directories/{id} -- get a single watched directory.
#[utoipa::path(
    get,
    path = "/api/watched-directories/{id}",
    tag = "watched-directories",
    params(("id" = Uuid, Path, description = "Watched directory ID")),
    responses(
        (status = 200, description = "Watched directory details", body = WatchedDirectoryResponse),
        (status = 404, description = "Watched directory not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn get_watched(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    AxumPath(id): AxumPath<Uuid>,
) -> Result<Json<WatchedDirectoryResponse>, ApiError> {
    let dir = WatchedDirectoryRepository::get_by_id(state.db_pool(), id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("watched directory not found: {id}")))?;

    let detection = detect_fs_type(Path::new(&dir.path));
    Ok(Json(to_response(
        &dir,
        Some(to_fs_detection_response(&detection)),
    )))
}

/// PUT /api/watched-directories/{id} -- update a watched directory.
///
/// Can change `watch_mode`, `poll_interval_secs`, and `enabled`.
/// Cannot change `path` (delete and re-create instead).
#[utoipa::path(
    put,
    path = "/api/watched-directories/{id}",
    tag = "watched-directories",
    params(("id" = Uuid, Path, description = "Watched directory ID")),
    request_body = UpdateWatchedDirectoryRequest,
    responses(
        (status = 200, description = "Updated watched directory", body = WatchedDirectoryResponse),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Watched directory not found"),
        (status = 401, description = "Not authenticated"),
        (status = 503, description = "Watcher subsystem disabled"),
    ),
    security(("bearer" = []))
)]
pub async fn update_watched(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    AxumPath(id): AxumPath<Uuid>,
    Json(body): Json<UpdateWatchedDirectoryRequest>,
) -> Result<Json<WatchedDirectoryResponse>, ApiError> {
    require_watcher_enabled(&state)?;

    // Load existing to know the current state.
    let existing = WatchedDirectoryRepository::get_by_id(state.db_pool(), id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("watched directory not found: {id}")))?;

    let new_mode: Option<WatchMode> = body
        .watch_mode
        .as_deref()
        .map(|s| s.parse().map_err(|e: String| ApiError::Validation(e)))
        .transpose()?;

    let updated = WatchedDirectoryRepository::update(
        state.db_pool(),
        id,
        new_mode,
        body.poll_interval_secs,
        body.enabled,
    )
    .await?;

    // Manage watcher lifecycle based on changes.
    if let Some(ws) = state.watcher_service() {
        let mode_changed = new_mode.is_some() && new_mode != Some(existing.watch_mode);
        let enabled_changed = body.enabled.is_some() && body.enabled != Some(existing.enabled);

        if mode_changed {
            // Watch mode changed: unwatch old, watch with new mode.
            ws.read()
                .await
                .unwatch(Path::new(&existing.path))
                .await
                .ok();
            if updated.enabled {
                ws.read().await.watch(&updated).await.ok();
            }
        } else if enabled_changed {
            if updated.enabled {
                ws.read().await.watch(&updated).await.ok();
            } else {
                ws.read()
                    .await
                    .unwatch(Path::new(&existing.path))
                    .await
                    .ok();
            }
        }
    }

    let detection = detect_fs_type(Path::new(&updated.path));
    Ok(Json(to_response(
        &updated,
        Some(to_fs_detection_response(&detection)),
    )))
}

/// DELETE /api/watched-directories/{id} -- remove a watched directory.
///
/// Stops watching and deletes the database record. Does NOT delete any
/// previously imported books.
#[utoipa::path(
    delete,
    path = "/api/watched-directories/{id}",
    tag = "watched-directories",
    params(("id" = Uuid, Path, description = "Watched directory ID")),
    responses(
        (status = 204, description = "Watched directory deleted"),
        (status = 404, description = "Watched directory not found"),
        (status = 401, description = "Not authenticated"),
        (status = 503, description = "Watcher subsystem disabled"),
    ),
    security(("bearer" = []))
)]
pub async fn remove_watched(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    AxumPath(id): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_watcher_enabled(&state)?;

    let dir = WatchedDirectoryRepository::get_by_id(state.db_pool(), id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("watched directory not found: {id}")))?;

    // Stop watching before deleting.
    if let Some(ws) = state.watcher_service() {
        ws.read().await.unwatch(Path::new(&dir.path)).await.ok();
    }

    WatchedDirectoryRepository::delete(state.db_pool(), id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/watched-directories/{id}/scan -- trigger a manual full scan.
///
/// Enqueues an `ImportDirectory` background task for the watched directory's path.
/// Returns 202 with the task ID for progress tracking.
#[utoipa::path(
    post,
    path = "/api/watched-directories/{id}/scan",
    tag = "watched-directories",
    params(("id" = Uuid, Path, description = "Watched directory ID")),
    responses(
        (status = 202, description = "Scan task enqueued", body = ScanTriggeredResponse),
        (status = 404, description = "Watched directory not found"),
        (status = 401, description = "Not authenticated"),
        (status = 503, description = "Watcher subsystem disabled"),
    ),
    security(("bearer" = []))
)]
pub async fn trigger_scan(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    AxumPath(id): AxumPath<Uuid>,
) -> Result<(StatusCode, Json<ScanTriggeredResponse>), ApiError> {
    require_watcher_enabled(&state)?;

    let dir = WatchedDirectoryRepository::get_by_id(state.db_pool(), id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("watched directory not found: {id}")))?;

    let payload = serde_json::json!({
        "directory_path": dir.path,
    });

    let task_id = state
        .task_queue()
        .enqueue(TaskType::ImportDirectory, payload)
        .await
        .map_err(|e| ApiError::Core(archivis_core::errors::ArchivisError::from(e)))?;

    Ok((
        StatusCode::ACCEPTED,
        Json(ScanTriggeredResponse { task_id }),
    ))
}

/// POST /api/watched-directories/detect -- detect filesystem type for a path.
///
/// Validates the path and returns a filesystem detection hint without adding
/// it as a watched directory. Used by the frontend to show the recommendation
/// before the user finalizes their watch mode choice.
#[utoipa::path(
    post,
    path = "/api/watched-directories/detect",
    tag = "watched-directories",
    request_body = DetectFsRequest,
    responses(
        (status = 200, description = "Filesystem detection result", body = FsDetectionResponse),
        (status = 400, description = "Invalid or inaccessible path"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn detect_fs(
    State(_state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<DetectFsRequest>,
) -> Result<Json<FsDetectionResponse>, ApiError> {
    let canonical = validate_directory_path(&body.path).await?;
    let detection = detect_fs_type(&canonical);
    Ok(Json(to_fs_detection_response(&detection)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Path validation ────────────────────────────────────────────

    #[tokio::test]
    async fn validate_directory_path_rejects_relative() {
        let result = validate_directory_path("relative/path").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(&err, ApiError::Validation(msg) if msg.contains("absolute")),
            "expected absolute path error, got: {err}"
        );
    }

    #[tokio::test]
    async fn validate_directory_path_rejects_nonexistent() {
        let result = validate_directory_path("/nonexistent_path_9f8a7b6c5d4e").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(&err, ApiError::Validation(msg) if msg.contains("cannot access")),
            "expected access error, got: {err}"
        );
    }

    #[tokio::test]
    async fn validate_directory_path_rejects_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("not-a-dir.txt");
        std::fs::write(&file_path, "hello").unwrap();
        let result = validate_directory_path(file_path.to_str().unwrap()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(&err, ApiError::Validation(msg) if msg.contains("not a directory")),
            "expected not-a-directory error, got: {err}"
        );
    }

    #[tokio::test]
    async fn validate_directory_path_accepts_real_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = validate_directory_path(tmp.path().to_str().unwrap()).await;
        assert!(result.is_ok());
    }

    // ── Storage path validation ────────────────────────────────────

    #[test]
    fn validate_not_inside_storage_rejects_subpath() {
        let storage_root = Path::new("/data/books");
        let watched = Path::new("/data/books/incoming");
        let result = validate_not_inside_storage(watched, storage_root);
        assert!(result.is_err());
    }

    #[test]
    fn validate_not_inside_storage_accepts_sibling() {
        let storage_root = Path::new("/data/books");
        let watched = Path::new("/data/incoming");
        let result = validate_not_inside_storage(watched, storage_root);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_not_inside_storage_rejects_exact_path() {
        let storage_root = Path::new("/data/books");
        let watched = Path::new("/data/books");
        let result = validate_not_inside_storage(watched, storage_root);
        assert!(result.is_err());
    }

    // ── Response building ──────────────────────────────────────────

    #[test]
    fn to_response_uses_default_poll_interval() {
        let dir = archivis_core::models::WatchedDirectory {
            id: Uuid::new_v4(),
            path: "/tmp/test".to_string(),
            watch_mode: WatchMode::Poll,
            poll_interval_secs: None,
            enabled: true,
            last_error: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let resp = to_response(&dir, None);
        assert_eq!(
            resp.effective_poll_interval_secs,
            DEFAULT_POLL_INTERVAL_SECS
        );
        assert!(resp.poll_interval_secs.is_none());
    }

    #[test]
    fn to_response_uses_per_directory_interval() {
        let dir = archivis_core::models::WatchedDirectory {
            id: Uuid::new_v4(),
            path: "/tmp/test".to_string(),
            watch_mode: WatchMode::Poll,
            poll_interval_secs: Some(60),
            enabled: true,
            last_error: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let resp = to_response(&dir, None);
        assert_eq!(resp.effective_poll_interval_secs, 60);
        assert_eq!(resp.poll_interval_secs, Some(60));
    }

    #[test]
    fn to_response_includes_detection() {
        let dir = archivis_core::models::WatchedDirectory {
            id: Uuid::new_v4(),
            path: "/tmp/test".to_string(),
            watch_mode: WatchMode::Native,
            poll_interval_secs: None,
            enabled: true,
            last_error: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let detection = FsDetectionResponse {
            fs_type: "ext4".to_string(),
            native_likely_works: "likely".to_string(),
            explanation: "Local filesystem.".to_string(),
        };
        let resp = to_response(&dir, Some(detection));
        assert!(resp.detected_fs.is_some());
        let fs = resp.detected_fs.unwrap();
        assert_eq!(fs.fs_type, "ext4");
        assert_eq!(fs.native_likely_works, "likely");
    }

    #[test]
    fn to_response_watch_mode_lowercase() {
        let dir = archivis_core::models::WatchedDirectory {
            id: Uuid::new_v4(),
            path: "/tmp/test".to_string(),
            watch_mode: WatchMode::Native,
            poll_interval_secs: None,
            enabled: true,
            last_error: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let resp = to_response(&dir, None);
        assert_eq!(resp.watch_mode, "native");
    }
}
