//! Event processor for the filesystem watcher subsystem.
//!
//! Consumes [`WatcherEvent`]s from the watcher service, deduplicates them,
//! and enqueues import tasks. Handles file removals by logging warnings for
//! orphaned `BookFile` records. Optionally deletes source files after
//! successful watcher-triggered imports.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use archivis_core::models::{TaskProgress, TaskStatus, TaskType};
use archivis_core::settings::{SettingsReader, SettingsReaderExt};
use archivis_db::{BookFileRepository, DbPool, WatchedDirectoryRepository};
use archivis_storage::watcher::WatcherEvent;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::queue::TaskQueue;

/// Cooldown period: events for the same path within this window are deduplicated.
const DEDUP_COOLDOWN: Duration = Duration::from_secs(5);

/// TTL for tracking recently-deleted source files (to suppress spurious
/// `FileRemoved` events caused by the processor itself).
const DELETE_SUPPRESSION_TTL: Duration = Duration::from_secs(30);

/// Run the watcher event processing loop.
///
/// This function runs as a long-lived task (via `tokio::spawn`). It:
/// 1. Receives events from the watcher service
/// 2. Deduplicates rapid events for the same path
/// 3. Enqueues `ImportFile` tasks for new/changed files
/// 4. Logs warnings for removed files that have matching `BookFile` records
/// 5. Persists watcher errors to the `watched_directories.last_error` column
/// 6. Optionally deletes source files after successful watcher-triggered imports
pub async fn run(
    mut event_rx: mpsc::Receiver<WatcherEvent>,
    task_queue: Arc<TaskQueue>,
    db_pool: DbPool,
    settings: Arc<dyn SettingsReader>,
) {
    info!("watcher event processor started");

    let mut dedup_map: HashMap<PathBuf, Instant> = HashMap::new();

    // Track watcher-sourced import tasks for delete-source-after-import.
    // Maps task_id -> source file path.
    let mut pending_watcher_imports: HashMap<Uuid, PathBuf> = HashMap::new();

    // Recently-deleted source files: suppress FileRemoved events for these.
    let mut recently_deleted: HashMap<PathBuf, Instant> = HashMap::new();

    // Subscribe to task progress to detect completed watcher-sourced imports.
    let mut progress_rx = task_queue.subscribe_all();

    loop {
        tokio::select! {
            event = event_rx.recv() => {
                let Some(event) = event else {
                    info!("watcher event channel closed, processor stopping");
                    break;
                };

                process_event(
                    event,
                    &task_queue,
                    &db_pool,
                    &mut dedup_map,
                    &mut pending_watcher_imports,
                    &recently_deleted,
                )
                .await;
            }

            progress = progress_rx.recv() => {
                match progress {
                    Ok(update) => {
                        handle_task_completion(
                            &update,
                            settings.as_ref(),
                            &mut pending_watcher_imports,
                            &mut recently_deleted,
                        )
                        .await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "watcher processor lagged on task progress channel");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("task progress channel closed, processor stopping");
                        break;
                    }
                }
            }
        }

        // Periodic cleanup of stale dedup entries and suppression entries.
        cleanup_stale_entries(&mut dedup_map, &mut recently_deleted);
    }

    info!("watcher event processor stopped");
}

/// Process a single watcher event.
async fn process_event(
    event: WatcherEvent,
    task_queue: &TaskQueue,
    db_pool: &DbPool,
    dedup_map: &mut HashMap<PathBuf, Instant>,
    pending_watcher_imports: &mut HashMap<Uuid, PathBuf>,
    recently_deleted: &HashMap<PathBuf, Instant>,
) {
    match event {
        WatcherEvent::FileChanged { path } => {
            handle_file_changed(
                &path,
                task_queue,
                db_pool,
                dedup_map,
                pending_watcher_imports,
            )
            .await;
        }
        WatcherEvent::FileRemoved { path } => {
            handle_file_removed(&path, db_pool, recently_deleted).await;
        }
        WatcherEvent::Error { error, path } => {
            handle_error(&error, path.as_deref(), db_pool).await;
        }
    }
}

/// Handle a `FileChanged` event: deduplicate, check for existing files, enqueue import.
async fn handle_file_changed(
    path: &PathBuf,
    task_queue: &TaskQueue,
    db_pool: &DbPool,
    dedup_map: &mut HashMap<PathBuf, Instant>,
    pending_watcher_imports: &mut HashMap<Uuid, PathBuf>,
) {
    // Deduplication: skip if we processed this path recently.
    let now = Instant::now();
    if let Some(last_processed) = dedup_map.get(path) {
        if now.duration_since(*last_processed) < DEDUP_COOLDOWN {
            debug!(path = %path.display(), "skipping duplicate event within cooldown window");
            return;
        }
    }

    // Check if the file still exists (may have been a transient temp file).
    if !path.exists() {
        debug!(path = %path.display(), "file no longer exists, skipping");
        return;
    }

    // Check if a BookFile with the same storage_path already exists.
    let path_str = path.to_string_lossy().to_string();
    match BookFileRepository::get_by_storage_path(db_pool, &path_str).await {
        Ok(existing_files) if !existing_files.is_empty() => {
            // File already known — compare hash to detect modifications.
            let current_hash = match compute_file_hash(path).await {
                Ok(h) => h,
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "failed to hash file, skipping");
                    return;
                }
            };

            let all_match = existing_files.iter().all(|f| f.hash == current_hash);
            if all_match {
                debug!(
                    path = %path.display(),
                    "file hash unchanged, skipping re-import"
                );
                dedup_map.insert(path.clone(), now);
                return;
            }

            // Hash changed — file was modified in place. Enqueue re-import.
            info!(
                path = %path.display(),
                "file modified (hash changed), enqueuing re-import"
            );
        }
        Ok(_) => {
            // No existing BookFile — new file.
            debug!(path = %path.display(), "new file detected");
        }
        Err(e) => {
            warn!(
                path = %path.display(),
                error = %e,
                "failed to check existing BookFile records"
            );
            // Continue with import attempt anyway.
        }
    }

    // Enqueue ImportFile task.
    let payload = serde_json::json!({
        "file_path": path_str,
        "source": "watcher",
    });

    match task_queue.enqueue(TaskType::ImportFile, payload).await {
        Ok(task_id) => {
            info!(
                %task_id,
                path = %path.display(),
                "enqueued watcher import task"
            );

            // Track for delete-source-after-import.
            pending_watcher_imports.insert(task_id, path.clone());
        }
        Err(e) => {
            error!(
                path = %path.display(),
                error = %e,
                "failed to enqueue watcher import task"
            );
        }
    }

    dedup_map.insert(path.clone(), now);
}

/// Handle a `FileRemoved` event: look up matching `BookFile` records and warn.
async fn handle_file_removed(
    path: &PathBuf,
    db_pool: &DbPool,
    recently_deleted: &HashMap<PathBuf, Instant>,
) {
    // Suppress FileRemoved events for files we just deleted (delete-source-after-import).
    if let Some(deleted_at) = recently_deleted.get(path) {
        if deleted_at.elapsed() < DELETE_SUPPRESSION_TTL {
            debug!(
                path = %path.display(),
                "suppressing FileRemoved event for recently-deleted source file"
            );
            return;
        }
    }

    let path_str = path.to_string_lossy().to_string();
    match BookFileRepository::get_by_storage_path(db_pool, &path_str).await {
        Ok(files) if !files.is_empty() => {
            for file in &files {
                warn!(
                    path = %path.display(),
                    book_file_id = %file.id,
                    book_id = %file.book_id,
                    "watched file removed — BookFile record may be orphaned"
                );
            }
        }
        Ok(_) => {
            debug!(
                path = %path.display(),
                "removed file has no matching BookFile records"
            );
        }
        Err(e) => {
            warn!(
                path = %path.display(),
                error = %e,
                "failed to look up BookFile records for removed file"
            );
        }
    }
}

/// Handle a watcher `Error` event: classify, log, and persist to
/// `watched_directories.last_error`.
///
/// Error classification:
/// - **Directory disappeared** (unmount, deletion): logged at `error` level.
///   The DB record is kept — the directory may reappear (NFS remount, USB
///   reconnect).
/// - **Permission denied**: logged at `warn` level with the specific subpath.
/// - **Other errors**: logged at `error` level with full context.
async fn handle_error(error: &str, path: Option<&std::path::Path>, db_pool: &DbPool) {
    let lower = error.to_lowercase();

    if let Some(path) = path {
        if is_path_gone_error(&lower) {
            error!(
                path = %path.display(),
                error = %error,
                "watched directory disappeared — it may reappear if remounted; \
                 keeping DB record"
            );
        } else if is_permission_error(&lower) {
            warn!(
                path = %path.display(),
                error = %error,
                "permission denied accessing watched path"
            );
        } else {
            error!(
                path = %path.display(),
                error = %error,
                "watcher error"
            );
        }

        // Persist the error to the matching watched directory's last_error column.
        if let Err(e) = persist_watcher_error(db_pool, path, error).await {
            warn!(
                path = %path.display(),
                error = %e,
                "failed to persist watcher error to database"
            );
        }
    } else {
        error!(error = %error, "watcher error (no path context)");
    }
}

/// Returns `true` if the error indicates a watched path no longer exists
/// (e.g., unmounted NFS share, deleted directory).
fn is_path_gone_error(lower_error: &str) -> bool {
    lower_error.contains("no such file")
        || lower_error.contains("not found")
        || lower_error.contains("enoent")
        || lower_error.contains("does not exist")
}

/// Returns `true` if the error indicates a permission denial.
fn is_permission_error(lower_error: &str) -> bool {
    lower_error.contains("permission denied")
        || lower_error.contains("access denied")
        || lower_error.contains("eperm")
        || lower_error.contains("eacces")
}

/// Persist a watcher error to the matching watched directory's `last_error` column.
async fn persist_watcher_error(
    db_pool: &DbPool,
    path: &std::path::Path,
    error: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path_str = path.to_string_lossy().to_string();

    // Find which watched directory this path belongs to.
    let directories = WatchedDirectoryRepository::list_all(db_pool).await?;
    for dir in &directories {
        if path_str.starts_with(&dir.path) {
            WatchedDirectoryRepository::set_last_error(db_pool, dir.id, Some(error)).await?;
            return Ok(());
        }
    }

    Ok(())
}

/// Handle a completed watcher-sourced import task: optionally delete the source file.
async fn handle_task_completion(
    update: &TaskProgress,
    settings: &dyn SettingsReader,
    pending_watcher_imports: &mut HashMap<Uuid, PathBuf>,
    recently_deleted: &mut HashMap<PathBuf, Instant>,
) {
    // Only process terminal task states.
    if !update.status.is_terminal() {
        return;
    }

    // Check if this is a watcher-sourced import.
    let Some(source_path) = pending_watcher_imports.remove(&update.task_id) else {
        return;
    };

    // Only delete source on successful completion.
    if update.status != TaskStatus::Completed {
        debug!(
            task_id = %update.task_id,
            status = %update.status,
            path = %source_path.display(),
            "watcher import did not complete successfully, keeping source file"
        );
        return;
    }

    // Check the import result — only delete if the file was actually imported
    // (not skipped as a duplicate with the same hash).
    if let Some(result) = &update.result {
        let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if status == "skipped_duplicate" || status == "duplicate" {
            debug!(
                task_id = %update.task_id,
                path = %source_path.display(),
                "import skipped as duplicate, keeping source file"
            );
            return;
        }
    }

    // PerUse: re-read the current setting on every completion.
    if !settings
        .get_bool("watcher.delete_source_after_import")
        .unwrap_or(false)
    {
        return;
    }

    // Delete the source file.
    match tokio::fs::remove_file(&source_path).await {
        Ok(()) => {
            info!(
                task_id = %update.task_id,
                path = %source_path.display(),
                "deleted source file after successful import"
            );
            // Track deletion to suppress the resulting FileRemoved event.
            recently_deleted.insert(source_path, Instant::now());
        }
        Err(e) => {
            warn!(
                task_id = %update.task_id,
                path = %source_path.display(),
                error = %e,
                "failed to delete source file after import"
            );
        }
    }
}

/// Remove stale entries from the dedup map and recently-deleted set.
fn cleanup_stale_entries(
    dedup_map: &mut HashMap<PathBuf, Instant>,
    recently_deleted: &mut HashMap<PathBuf, Instant>,
) {
    let now = Instant::now();

    dedup_map.retain(|_, last| now.duration_since(*last) < DEDUP_COOLDOWN * 2);
    recently_deleted
        .retain(|_, deleted_at| now.duration_since(*deleted_at) < DELETE_SUPPRESSION_TTL);
}

/// Compute SHA-256 hash of a file, returning the hex-encoded string.
async fn compute_file_hash(path: &std::path::Path) -> Result<String, std::io::Error> {
    let data = tokio::fs::read(path).await?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let result = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in result {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Dedup logic ─────────────────────────────────────────────────

    #[test]
    fn dedup_within_cooldown() {
        let mut dedup_map: HashMap<PathBuf, Instant> = HashMap::new();
        let path = PathBuf::from("/books/test.epub");
        let now = Instant::now();

        // First event: not deduplicated.
        assert!(!is_deduped(&path, &dedup_map, now));
        dedup_map.insert(path.clone(), now);

        // Second event within cooldown: deduplicated.
        let within_cooldown = now + Duration::from_secs(2);
        assert!(is_deduped(&path, &dedup_map, within_cooldown));

        // Third event after cooldown: not deduplicated.
        let after_cooldown = now + DEDUP_COOLDOWN + Duration::from_millis(1);
        assert!(!is_deduped(&path, &dedup_map, after_cooldown));
    }

    #[test]
    fn dedup_different_paths_independent() {
        let mut dedup_map: HashMap<PathBuf, Instant> = HashMap::new();
        let path_a = PathBuf::from("/books/a.epub");
        let path_b = PathBuf::from("/books/b.epub");
        let now = Instant::now();

        dedup_map.insert(path_a.clone(), now);

        // Different path is not deduplicated.
        assert!(!is_deduped(&path_b, &dedup_map, now));
        // Same path is deduplicated.
        assert!(is_deduped(&path_a, &dedup_map, now));
    }

    /// Helper to check dedup logic (mirrors the check in `handle_file_changed`).
    fn is_deduped(path: &PathBuf, dedup_map: &HashMap<PathBuf, Instant>, now: Instant) -> bool {
        dedup_map
            .get(path)
            .is_some_and(|last_processed| now.duration_since(*last_processed) < DEDUP_COOLDOWN)
    }

    // ── Cleanup ─────────────────────────────────────────────────────

    #[test]
    fn cleanup_removes_stale_dedup_entries() {
        let mut dedup_map: HashMap<PathBuf, Instant> = HashMap::new();
        let mut recently_deleted: HashMap<PathBuf, Instant> = HashMap::new();

        // Use different stale times: dedup_stale must exceed DEDUP_COOLDOWN * 2,
        // delete_stale must exceed DELETE_SUPPRESSION_TTL.
        let dedup_stale_time = Instant::now().checked_sub(DEDUP_COOLDOWN * 3).unwrap();
        let delete_stale_time = Instant::now()
            .checked_sub(DELETE_SUPPRESSION_TTL * 2)
            .unwrap();
        let fresh_time = Instant::now();

        dedup_map.insert(PathBuf::from("/stale.epub"), dedup_stale_time);
        dedup_map.insert(PathBuf::from("/fresh.epub"), fresh_time);

        recently_deleted.insert(PathBuf::from("/stale-del.epub"), delete_stale_time);
        recently_deleted.insert(PathBuf::from("/fresh-del.epub"), fresh_time);

        cleanup_stale_entries(&mut dedup_map, &mut recently_deleted);

        assert_eq!(dedup_map.len(), 1);
        assert!(dedup_map.contains_key(&PathBuf::from("/fresh.epub")));

        assert_eq!(recently_deleted.len(), 1);
        assert!(recently_deleted.contains_key(&PathBuf::from("/fresh-del.epub")));
    }

    // ── Delete suppression ──────────────────────────────────────────

    #[test]
    fn recently_deleted_suppresses_file_removed() {
        let mut recently_deleted: HashMap<PathBuf, Instant> = HashMap::new();
        let path = PathBuf::from("/books/imported.epub");

        // Not recently deleted — should NOT be suppressed.
        assert!(!is_suppressed(&path, &recently_deleted));

        // Recently deleted — should be suppressed.
        recently_deleted.insert(path.clone(), Instant::now());
        assert!(is_suppressed(&path, &recently_deleted));
    }

    #[test]
    fn suppression_expires_after_ttl() {
        let mut recently_deleted: HashMap<PathBuf, Instant> = HashMap::new();
        let path = PathBuf::from("/books/imported.epub");

        // Deleted long ago — should NOT be suppressed.
        recently_deleted.insert(
            path.clone(),
            Instant::now()
                .checked_sub(DELETE_SUPPRESSION_TTL * 2)
                .unwrap(),
        );
        assert!(!is_suppressed(&path, &recently_deleted));
    }

    /// Helper mirroring the suppression check in `handle_file_removed`.
    fn is_suppressed(path: &PathBuf, recently_deleted: &HashMap<PathBuf, Instant>) -> bool {
        recently_deleted
            .get(path)
            .is_some_and(|deleted_at| deleted_at.elapsed() < DELETE_SUPPRESSION_TTL)
    }

    // ── File hash ───────────────────────────────────────────────────

    #[tokio::test]
    async fn compute_file_hash_works() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        tokio::fs::write(&file_path, b"hello world").await.unwrap();

        let hash = compute_file_hash(&file_path).await.unwrap();
        // SHA-256 of "hello world"
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[tokio::test]
    async fn compute_file_hash_nonexistent_file() {
        let result = compute_file_hash(std::path::Path::new("/nonexistent/file.epub")).await;
        assert!(result.is_err());
    }

    // ── Error classification ─────────────────────────────────────────

    #[test]
    fn path_gone_error_detection() {
        // Note: these functions receive already-lowercased strings from `handle_error`.
        assert!(is_path_gone_error("no such file or directory"));
        assert!(is_path_gone_error("path not found: /mnt/nfs/books"));
        assert!(is_path_gone_error("enoent: entity does not exist"));
        assert!(is_path_gone_error("directory does not exist"));
        assert!(!is_path_gone_error("permission denied"));
        assert!(!is_path_gone_error("inotify watch limit reached"));
    }

    #[test]
    fn permission_error_detection() {
        // Note: these functions receive already-lowercased strings from `handle_error`.
        assert!(is_permission_error("permission denied"));
        assert!(is_permission_error("access denied to /books/private"));
        assert!(is_permission_error("eperm: operation not permitted"));
        assert!(is_permission_error("eacces: permission denied"));
        assert!(!is_permission_error("no such file or directory"));
        assert!(!is_permission_error("inotify limit reached"));
    }
}
