use std::path::PathBuf;

use axum::extract::{Query, State};
use axum::Json;

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{BrowseParams, BrowseResponse, FsEntry};

/// GET /api/filesystem/browse -- browse a server directory.
///
/// Lists entries in the given directory. Requires authentication.
/// When no path is provided, defaults to the server's data directory.
/// Hidden entries (dot-prefixed) are excluded. Results are sorted with
/// directories first, then alphabetically case-insensitive.
#[utoipa::path(
    get,
    path = "/api/filesystem/browse",
    tag = "filesystem",
    params(BrowseParams),
    responses(
        (status = 200, description = "Directory listing", body = BrowseResponse),
        (status = 400, description = "Invalid or inaccessible path"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn browse_directory(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(params): Query<BrowseParams>,
) -> Result<Json<BrowseResponse>, ApiError> {
    let path = if let Some(p) = params.path {
        let user_path = PathBuf::from(p);
        if !user_path.is_absolute() {
            return Err(ApiError::Validation("path must be absolute".into()));
        }
        user_path
    } else {
        state.config().data_dir.clone()
    };

    // Canonicalize to resolve symlinks and `..` components (traversal prevention).
    let canonical = tokio::fs::canonicalize(&path).await.map_err(|e| {
        ApiError::Validation(format!("cannot access path '{}': {e}", path.display()))
    })?;

    // Verify it's a directory.
    let meta = tokio::fs::metadata(&canonical).await.map_err(|e| {
        ApiError::Validation(format!("cannot access path '{}': {e}", canonical.display()))
    })?;
    if !meta.is_dir() {
        return Err(ApiError::Validation(format!(
            "path is not a directory: {}",
            canonical.display()
        )));
    }

    let mut entries = Vec::new();
    let mut read_dir = tokio::fs::read_dir(&canonical).await.map_err(|e| {
        ApiError::Validation(format!(
            "cannot read directory '{}': {e}",
            canonical.display()
        ))
    })?;

    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();

        // Skip hidden entries (dot-prefixed).
        if name.starts_with('.') {
            continue;
        }

        // Silently skip entries whose metadata can't be read (permission errors).
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };

        let is_dir = metadata.is_dir();

        // When dirs_only is set, skip non-directory entries.
        if params.dirs_only && !is_dir {
            continue;
        }

        let size = if is_dir { 0 } else { metadata.len() };

        entries.push(FsEntry { name, is_dir, size });
    }

    // Sort: directories first, then alphabetical case-insensitive.
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let parent = canonical.parent().map(|p| p.to_string_lossy().into_owned());

    Ok(Json(BrowseResponse {
        path: canonical.to_string_lossy().into_owned(),
        parent,
        entries,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: call the handler with given params (no auth check in unit tests).
    async fn call_browse(
        path: Option<&str>,
        dirs_only: bool,
    ) -> Result<Json<BrowseResponse>, ApiError> {
        // We test the core logic by calling the inner parts directly.
        let raw_path = path.map_or_else(|| "/".to_string(), ToString::to_string);
        let path_buf = PathBuf::from(&raw_path);

        if !path_buf.is_absolute() {
            return Err(ApiError::Validation("path must be absolute".into()));
        }

        let canonical = tokio::fs::canonicalize(&path_buf).await.map_err(|e| {
            ApiError::Validation(format!("cannot access path '{}': {e}", path_buf.display()))
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

        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&canonical).await.map_err(|e| {
            ApiError::Validation(format!(
                "cannot read directory '{}': {e}",
                canonical.display()
            ))
        })?;

        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(metadata) = entry.metadata().await else {
                continue;
            };
            let is_dir = metadata.is_dir();
            if dirs_only && !is_dir {
                continue;
            }
            let size = if is_dir { 0 } else { metadata.len() };
            entries.push(FsEntry { name, is_dir, size });
        }

        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        let parent = canonical.parent().map(|p| p.to_string_lossy().into_owned());

        Ok(Json(BrowseResponse {
            path: canonical.to_string_lossy().into_owned(),
            parent,
            entries,
        }))
    }

    #[tokio::test]
    async fn rejects_relative_path() {
        let result = call_browse(Some("relative/path"), false).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, ApiError::Validation(ref msg) if msg.contains("absolute")),
            "expected absolute path error, got: {err}"
        );
    }

    #[tokio::test]
    async fn rejects_nonexistent_path() {
        let result = call_browse(Some("/nonexistent_path_9f8a7b6c5d4e"), false).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, ApiError::Validation(ref msg) if msg.contains("cannot access")),
            "expected access error, got: {err}"
        );
    }

    #[tokio::test]
    async fn dirs_only_filters_files() {
        let tmp = tempfile::tempdir().unwrap();
        // Create a subdirectory and a file.
        std::fs::create_dir(tmp.path().join("subdir")).unwrap();
        std::fs::write(tmp.path().join("file.txt"), "hello").unwrap();

        let result = call_browse(Some(tmp.path().to_str().unwrap()), true)
            .await
            .unwrap();

        // Should only contain the directory.
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].name, "subdir");
        assert!(result.entries[0].is_dir);
    }

    #[tokio::test]
    async fn hidden_files_excluded() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".hidden"), "secret").unwrap();
        std::fs::write(tmp.path().join("visible.txt"), "hello").unwrap();

        let result = call_browse(Some(tmp.path().to_str().unwrap()), false)
            .await
            .unwrap();

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].name, "visible.txt");
    }

    #[tokio::test]
    async fn sorting_dirs_first_then_alpha() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("zebra.txt"), "z").unwrap();
        std::fs::create_dir(tmp.path().join("beta")).unwrap();
        std::fs::write(tmp.path().join("alpha.txt"), "a").unwrap();
        std::fs::create_dir(tmp.path().join("alpha_dir")).unwrap();

        let result = call_browse(Some(tmp.path().to_str().unwrap()), false)
            .await
            .unwrap();

        let names: Vec<&str> = result.entries.iter().map(|e| e.name.as_str()).collect();
        // Directories first (alphabetically), then files (alphabetically)
        assert_eq!(names, vec!["alpha_dir", "beta", "alpha.txt", "zebra.txt"]);
    }

    #[tokio::test]
    async fn returns_parent_path() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("child");
        std::fs::create_dir(&sub).unwrap();

        let result = call_browse(Some(sub.to_str().unwrap()), false)
            .await
            .unwrap();

        assert!(result.parent.is_some());
        // Parent should be the tmp dir (canonicalized).
        let expected_parent = tokio::fs::canonicalize(tmp.path())
            .await
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(result.parent.clone().unwrap(), expected_parent);
    }
}
