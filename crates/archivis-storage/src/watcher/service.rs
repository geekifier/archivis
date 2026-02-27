//! Core watcher service managing filesystem watcher lifecycle.
//!
//! The `WatcherService` orchestrates native (inotify/FSEvents/ReadDirectoryChanges) and
//! polling watchers per watched directory, merging debounced events into a single
//! channel for downstream processing.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use archivis_core::errors::StorageError;
use archivis_core::models::{WatchMode, WatchedDirectory};
use notify::RecursiveMode;
use notify_debouncer_full::{
    new_debouncer, new_debouncer_opt, DebounceEventResult, DebouncedEvent, Debouncer, NoCache,
    RecommendedCache,
};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

use super::WatcherEvent;

/// Supported file extensions for watcher event pre-filtering.
/// Matches `SUPPORTED_EXTENSIONS` in `archivis-tasks/src/import/bulk.rs`.
const SUPPORTED_EXTENSIONS: &[&str] = &["epub", "pdf", "mobi", "azw3", "cbz", "fb2", "txt", "djvu"];

/// Temporary file patterns to skip (incomplete downloads, editor swap files).
const TEMP_SUFFIXES: &[&str] = &[".part", ".tmp", ".crdownload", ".swp"];

/// Runtime watcher configuration sourced from the database settings table.
/// Separate from the boot-only `WatcherConfig` in `archivis-server`.
#[derive(Debug, Clone)]
pub struct WatcherRuntimeConfig {
    /// Debounce window in milliseconds. Events within this window are merged.
    pub debounce_ms: u64,
    /// Default polling interval in seconds for paths using poll mode.
    /// Per-directory overrides take precedence.
    pub default_poll_interval_secs: u64,
}

/// Internal state behind the `Arc<Mutex<>>` for `Send + Sync` safety.
struct WatcherInner {
    /// Debouncer for paths using native OS events. Wrapped in `Option` so
    /// `shutdown()` can drop it immediately to release OS resources.
    native_debouncer: Option<Debouncer<notify::RecommendedWatcher, RecommendedCache>>,
    /// Debouncer for paths using polling. Wrapped in `Option` for the same reason.
    poll_debouncer: Option<Debouncer<notify::PollWatcher, NoCache>>,
    /// Set of currently-watched paths and their assigned backend.
    watched_paths: HashSet<PathBuf>,
    /// Tracks which paths were assigned to the native backend (vs poll).
    native_paths: HashSet<PathBuf>,
    /// Runtime configuration (debounce window, default poll interval).
    config: WatcherRuntimeConfig,
}

/// Core watcher service managing the lifecycle of filesystem watchers.
///
/// Creates and manages native or polling watchers per watched directory,
/// filters raw events, and emits normalized `WatcherEvent`s.
///
/// This type is `Send + Sync` for storage in `AppState`.
#[derive(Clone)]
pub struct WatcherService {
    inner: Arc<Mutex<WatcherInner>>,
    /// Receiver handed out via `event_receiver()`. Wrapped in `Option` so it
    /// can be taken once.
    event_rx: Arc<Mutex<Option<mpsc::Receiver<WatcherEvent>>>>,
}

impl WatcherService {
    /// Create a new watcher service. Does not start watching — call `start()`.
    pub fn new(config: WatcherRuntimeConfig) -> Result<Self, StorageError> {
        let debounce_duration = Duration::from_millis(config.debounce_ms);

        // Tokio channel for emitting filtered WatcherEvents to the processor.
        let (event_tx, event_rx) = mpsc::channel::<WatcherEvent>(1024);

        // Move the sender into the debouncer callbacks (via clones).
        let tx_native = event_tx.clone();
        let tx_poll = event_tx;
        // Original `event_tx` is consumed above; debouncers own the senders.

        // Native debouncer — wraps RecommendedWatcher.
        let native_debouncer = new_debouncer(
            debounce_duration,
            None,
            move |result: DebounceEventResult| {
                handle_debounced_events(result, &tx_native);
            },
        )
        .map_err(|e| StorageError::Watcher(format!("failed to create native debouncer: {e}")))?;

        // Poll debouncer — wraps PollWatcher with a default interval.
        // The interval is per-directory, but notify's Config applies globally
        // to the PollWatcher instance. We use the global default here; per-path
        // overrides would require separate PollWatcher instances (future work).
        let poll_config = notify::Config::default()
            .with_poll_interval(Duration::from_secs(config.default_poll_interval_secs));

        let poll_debouncer = new_debouncer_opt::<_, notify::PollWatcher, NoCache>(
            debounce_duration,
            None,
            move |result: DebounceEventResult| {
                handle_debounced_events(result, &tx_poll);
            },
            NoCache,
            poll_config,
        )
        .map_err(|e| StorageError::Watcher(format!("failed to create poll debouncer: {e}")))?;

        Ok(Self {
            inner: Arc::new(Mutex::new(WatcherInner {
                native_debouncer: Some(native_debouncer),
                poll_debouncer: Some(poll_debouncer),
                watched_paths: HashSet::new(),
                native_paths: HashSet::new(),
                config,
            })),
            event_rx: Arc::new(Mutex::new(Some(event_rx))),
        })
    }

    /// Start watching the given directories. Called on server startup
    /// with the list from the database.
    pub async fn start(&self, directories: Vec<WatchedDirectory>) -> Result<(), StorageError> {
        for dir in &directories {
            if let Err(e) = self.watch(dir).await {
                error!(path = %dir.path, error = %e, "failed to start watching directory");
            }
        }
        Ok(())
    }

    /// Add a directory to watch at runtime (after API call).
    pub async fn watch(&self, directory: &WatchedDirectory) -> Result<(), StorageError> {
        let path = PathBuf::from(&directory.path);
        let mut inner = self.inner.lock().await;

        if inner.watched_paths.contains(&path) {
            debug!(path = %directory.path, "directory already watched, skipping");
            return Ok(());
        }

        match directory.watch_mode {
            WatchMode::Native => {
                let native = inner.native_debouncer.as_mut().ok_or_else(|| {
                    StorageError::Watcher("watcher service has been shut down".to_owned())
                })?;

                match native.watch(&path, RecursiveMode::Recursive) {
                    Ok(()) => {
                        info!(path = %directory.path, mode = "native", "watching directory");
                        inner.native_paths.insert(path.clone());
                    }
                    Err(e) => {
                        let err_str = e.to_string();

                        // Check for inotify watch limit exhaustion or permission errors
                        // that warrant a fallback to polling mode.
                        if is_inotify_limit_error(&err_str) {
                            error!(
                                path = %directory.path,
                                "inotify watch limit reached — either increase the limit \
                                 (`sysctl fs.inotify.max_user_watches=524288`) or switch this \
                                 path to polling mode; falling back to polling"
                            );

                            // Attempt fallback to poll mode for this path.
                            let poll = inner.poll_debouncer.as_mut().ok_or_else(|| {
                                StorageError::Watcher(
                                    "watcher service has been shut down".to_owned(),
                                )
                            })?;

                            poll.watch(&path, RecursiveMode::Recursive).map_err(|pe| {
                                StorageError::Watcher(format!(
                                    "failed to watch {} with polling fallback after inotify \
                                     limit: {pe}",
                                    path.display()
                                ))
                            })?;

                            info!(
                                path = %directory.path,
                                mode = "poll",
                                "watching directory (inotify fallback)"
                            );
                            // Note: not adding to native_paths since we fell back to poll.
                        } else if is_permission_error(&err_str) {
                            warn!(
                                path = %directory.path,
                                error = %e,
                                "permission denied watching directory, skipping"
                            );
                            return Err(StorageError::Watcher(format!(
                                "failed to watch {} with native events: {e}",
                                path.display()
                            )));
                        } else {
                            return Err(StorageError::Watcher(format!(
                                "failed to watch {} with native events: {e}",
                                path.display()
                            )));
                        }
                    }
                }
            }
            WatchMode::Poll => {
                let interval = directory
                    .poll_interval_secs
                    .map_or(inner.config.default_poll_interval_secs, i64::unsigned_abs);

                let poll = inner.poll_debouncer.as_mut().ok_or_else(|| {
                    StorageError::Watcher("watcher service has been shut down".to_owned())
                })?;

                // Note: notify's PollWatcher uses a single global poll interval
                // configured at creation time. Per-directory intervals would require
                // separate PollWatcher instances (future enhancement).
                poll.watch(&path, RecursiveMode::Recursive).map_err(|e| {
                    StorageError::Watcher(format!(
                        "failed to watch {} with polling: {e}",
                        path.display()
                    ))
                })?;
                info!(
                    path = %directory.path,
                    mode = "poll",
                    interval_secs = interval,
                    "watching directory"
                );
            }
        }

        inner.watched_paths.insert(path);
        drop(inner);
        Ok(())
    }

    /// Stop watching a directory at runtime.
    pub async fn unwatch(&self, path: &Path) -> Result<(), StorageError> {
        let mut inner = self.inner.lock().await;

        if !inner.watched_paths.remove(path) {
            debug!(path = %path.display(), "directory was not watched, nothing to unwatch");
            return Ok(());
        }

        // Try both debouncers — only one will have the path registered.
        // `unwatch` returns an error if the path isn't watched by that watcher,
        // which is expected for the "wrong" backend.
        let native_result = inner
            .native_debouncer
            .as_mut()
            .map_or(Ok(()), |d| d.unwatch(path));
        let poll_result = inner
            .poll_debouncer
            .as_mut()
            .map_or(Ok(()), |d| d.unwatch(path));

        inner.native_paths.remove(path);
        drop(inner);

        if native_result.is_err() && poll_result.is_err() {
            warn!(
                path = %path.display(),
                "failed to unwatch from both native and poll backends"
            );
        }

        info!(path = %path.display(), "unwatched directory");
        Ok(())
    }

    /// Returns the receiver for watcher events. The event loop (Task 4)
    /// consumes from this.
    ///
    /// This can only be called once — subsequent calls return `None`.
    pub async fn event_receiver(&self) -> Option<mpsc::Receiver<WatcherEvent>> {
        self.event_rx.lock().await.take()
    }

    /// Graceful shutdown — stop all watchers, drop debouncers to release OS
    /// resources, and drain remaining events from the channel.
    pub async fn shutdown(&self) {
        let mut inner = self.inner.lock().await;

        // Unwatch all paths from their respective backends.
        let paths: Vec<PathBuf> = inner.watched_paths.drain().collect();
        for path in &paths {
            if let Some(d) = inner.native_debouncer.as_mut() {
                let _ = d.unwatch(path);
            }
            if let Some(d) = inner.poll_debouncer.as_mut() {
                let _ = d.unwatch(path);
            }
        }
        inner.native_paths.clear();

        // Drop both debouncers to release OS watch handles and background threads.
        inner.native_debouncer.take();
        inner.poll_debouncer.take();
        drop(inner);

        // Drain any remaining events from the channel. The receiver may have
        // already been taken by the processor; if so, this is a no-op.
        let mut event_rx_guard = self.event_rx.lock().await;
        if let Some(rx) = event_rx_guard.as_mut() {
            let mut drained = 0usize;
            while rx.try_recv().is_ok() {
                drained += 1;
            }
            if drained > 0 {
                debug!(
                    count = drained,
                    "drained remaining watcher events during shutdown"
                );
            }
        }
        drop(event_rx_guard);

        info!("Filesystem watcher stopped");
    }
}

// ── Event handling and filtering ────────────────────────────────────────

/// Process debounced events from a debouncer callback, filter them, and
/// forward as `WatcherEvent`s through the tokio channel.
fn handle_debounced_events(result: DebounceEventResult, tx: &mpsc::Sender<WatcherEvent>) {
    match result {
        Ok(events) => {
            for debounced in &events {
                process_single_event(debounced, tx);
            }
        }
        Err(errors) => {
            for err in errors {
                let error_msg = err.to_string();
                let path = err.paths.first().cloned();
                let event = WatcherEvent::Error {
                    error: error_msg,
                    path,
                };
                if tx.try_send(event).is_err() {
                    warn!("watcher event channel full or closed, dropping error event");
                }
            }
        }
    }
}

/// Process a single debounced event, applying filters before emitting.
fn process_single_event(debounced: &DebouncedEvent, tx: &mpsc::Sender<WatcherEvent>) {
    let event = &debounced.event;

    for path in &event.paths {
        // Skip directories — only process files.
        if path.is_dir() {
            continue;
        }

        // Determine event type and apply filters.
        let watcher_event = if event.kind.is_create() || event.kind.is_modify() {
            if !should_process_file(path) {
                let reason = if is_hidden_file(path) {
                    "hidden file"
                } else if is_temp_file(path) {
                    "temporary file"
                } else {
                    "unsupported extension"
                };
                debug!(path = %path.display(), reason, "file event filtered");
                continue;
            }
            info!(path = %path.display(), "new file detected");
            WatcherEvent::FileChanged { path: path.clone() }
        } else if event.kind.is_remove() {
            // For removals, still apply extension filter but not temp file filter
            // (the file is gone, we can't check further).
            if !has_supported_extension(path) {
                debug!(path = %path.display(), reason = "unsupported extension", "file event filtered");
                continue;
            }
            WatcherEvent::FileRemoved { path: path.clone() }
        } else {
            // Other event kinds (Access, Other) — skip.
            continue;
        };

        if tx.try_send(watcher_event).is_err() {
            warn!(path = %path.display(), "watcher event channel full or closed, dropping event");
        }
    }
}

// ── Error classification helpers ─────────────────────────────────────

/// Returns `true` if the error indicates inotify watch limit exhaustion (ENOSPC).
fn is_inotify_limit_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("inotify") || lower.contains("enospc") || lower.contains("no space left")
}

/// Returns `true` if the error indicates a permission denial.
fn is_permission_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("permission denied")
        || lower.contains("access denied")
        || lower.contains("eperm")
}

// ── Filtering functions (public for testing) ─────────────────────────

/// Returns `true` if the file should be processed by the watcher.
///
/// Checks: not hidden, not temporary, has a supported extension.
pub(crate) fn should_process_file(path: &Path) -> bool {
    !is_hidden_file(path) && !is_temp_file(path) && has_supported_extension(path)
}

/// Returns `true` if the filename starts with `.` (hidden/dotfile).
pub(crate) fn is_hidden_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

/// Returns `true` if the file matches a temporary file pattern.
///
/// Patterns: `.part`, `.tmp`, `.crdownload`, `~` suffix, `.swp`.
pub(crate) fn is_temp_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };

    // Check ~ suffix (editor backup files).
    if name.ends_with('~') {
        return true;
    }

    // Check known temporary file suffixes.
    let lower = name.to_lowercase();
    TEMP_SUFFIXES.iter().any(|suffix| lower.ends_with(suffix))
}

/// Returns `true` if the file has a supported ebook extension.
pub(crate) fn has_supported_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── WatcherEvent construction and matching ────────────────────

    #[test]
    fn watcher_event_file_changed() {
        let event = WatcherEvent::FileChanged {
            path: PathBuf::from("/books/test.epub"),
        };
        assert!(matches!(event, WatcherEvent::FileChanged { .. }));
    }

    #[test]
    fn watcher_event_file_removed() {
        let event = WatcherEvent::FileRemoved {
            path: PathBuf::from("/books/test.epub"),
        };
        assert!(matches!(event, WatcherEvent::FileRemoved { .. }));
    }

    #[test]
    fn watcher_event_error() {
        let event = WatcherEvent::Error {
            error: "permission denied".to_owned(),
            path: Some(PathBuf::from("/books")),
        };
        assert!(matches!(event, WatcherEvent::Error { .. }));
    }

    #[test]
    fn watcher_event_error_without_path() {
        let event = WatcherEvent::Error {
            error: "generic error".to_owned(),
            path: None,
        };
        if let WatcherEvent::Error { path, .. } = event {
            assert!(path.is_none());
        } else {
            panic!("expected Error variant");
        }
    }

    // ── Extension filter ─────────────────────────────────────────

    #[test]
    fn accepts_supported_extensions() {
        let supported = ["epub", "pdf", "mobi", "azw3", "cbz", "fb2", "txt", "djvu"];
        for ext in supported {
            let path = PathBuf::from(format!("/books/test.{ext}"));
            assert!(
                has_supported_extension(&path),
                "expected {ext} to be accepted"
            );
        }
    }

    #[test]
    fn accepts_uppercase_extensions() {
        assert!(has_supported_extension(Path::new("/books/test.EPUB")));
        assert!(has_supported_extension(Path::new("/books/test.PDF")));
        assert!(has_supported_extension(Path::new("/books/test.Mobi")));
    }

    #[test]
    fn rejects_unsupported_extensions() {
        let unsupported = ["jpg", "png", "mp3", "nfo", "html", "css", "zip", "exe"];
        for ext in unsupported {
            let path = PathBuf::from(format!("/books/test.{ext}"));
            assert!(
                !has_supported_extension(&path),
                "expected {ext} to be rejected"
            );
        }
    }

    #[test]
    fn rejects_no_extension() {
        assert!(!has_supported_extension(Path::new("/books/README")));
        assert!(!has_supported_extension(Path::new("/books/Makefile")));
    }

    // ── Temporary file filter ────────────────────────────────────

    #[test]
    fn temp_file_part() {
        assert!(is_temp_file(Path::new("/downloads/book.epub.part")));
    }

    #[test]
    fn temp_file_tmp() {
        assert!(is_temp_file(Path::new("/downloads/book.tmp")));
    }

    #[test]
    fn temp_file_crdownload() {
        assert!(is_temp_file(Path::new("/downloads/book.epub.crdownload")));
    }

    #[test]
    fn temp_file_tilde_suffix() {
        assert!(is_temp_file(Path::new("/books/document.epub~")));
    }

    #[test]
    fn temp_file_swp() {
        assert!(is_temp_file(Path::new("/books/.document.swp")));
    }

    #[test]
    fn non_temp_file_epub() {
        assert!(!is_temp_file(Path::new("/books/document.epub")));
    }

    #[test]
    fn non_temp_file_pdf() {
        assert!(!is_temp_file(Path::new("/books/document.pdf")));
    }

    // ── Hidden file filter ───────────────────────────────────────

    #[test]
    fn hidden_file_dotfile() {
        assert!(is_hidden_file(Path::new("/books/.hidden-book.epub")));
    }

    #[test]
    fn hidden_file_dot_prefix() {
        assert!(is_hidden_file(Path::new("/books/.DS_Store")));
    }

    #[test]
    fn non_hidden_file() {
        assert!(!is_hidden_file(Path::new("/books/visible-book.epub")));
    }

    #[test]
    fn hidden_directory_child_not_hidden() {
        // The file itself is not hidden, even though a parent directory is.
        // We only check the filename, not the full path.
        assert!(!is_hidden_file(Path::new("/books/.hidden-dir/book.epub")));
    }

    // ── Combined filter ──────────────────────────────────────────

    #[test]
    fn should_process_normal_epub() {
        assert!(should_process_file(Path::new("/books/my-book.epub")));
    }

    #[test]
    fn should_not_process_hidden_epub() {
        assert!(!should_process_file(Path::new("/books/.my-book.epub")));
    }

    #[test]
    fn should_not_process_temp_epub() {
        assert!(!should_process_file(Path::new("/books/my-book.epub.part")));
    }

    #[test]
    fn should_not_process_jpg() {
        assert!(!should_process_file(Path::new("/books/cover.jpg")));
    }

    #[test]
    fn should_process_uppercase_pdf() {
        assert!(should_process_file(Path::new("/books/document.PDF")));
    }

    // ── WatcherRuntimeConfig ─────────────────────────────────────

    #[test]
    fn runtime_config_defaults() {
        let config = WatcherRuntimeConfig {
            debounce_ms: 2000,
            default_poll_interval_secs: 30,
        };
        assert_eq!(config.debounce_ms, 2000);
        assert_eq!(config.default_poll_interval_secs, 30);
    }

    // ── Error classification helpers ────────────────────────────

    #[test]
    fn inotify_limit_error_detection() {
        assert!(is_inotify_limit_error("inotify_add_watch returned ENOSPC"));
        assert!(is_inotify_limit_error("inotify watch limit reached"));
        assert!(is_inotify_limit_error("No space left on device (ENOSPC)"));
        assert!(is_inotify_limit_error("ENOSPC: no space left"));
        assert!(!is_inotify_limit_error("permission denied"));
        assert!(!is_inotify_limit_error("file not found"));
    }

    #[test]
    fn permission_error_detection() {
        assert!(is_permission_error("Permission denied"));
        assert!(is_permission_error("EPERM: operation not permitted"));
        assert!(is_permission_error("access denied to /books/private"));
        assert!(!is_permission_error("inotify watch limit reached"));
        assert!(!is_permission_error("file not found"));
    }

    // ── Shutdown behavior ───────────────────────────────────────

    #[tokio::test]
    async fn shutdown_clears_watched_paths() {
        let config = WatcherRuntimeConfig {
            debounce_ms: 500,
            default_poll_interval_secs: 10,
        };
        let service = WatcherService::new(config).unwrap();

        // Watch a real temp directory with poll mode (always works).
        let tmp = tempfile::tempdir().unwrap();
        let dir = archivis_core::models::WatchedDirectory {
            id: uuid::Uuid::new_v4(),
            path: tmp.path().to_string_lossy().to_string(),
            watch_mode: WatchMode::Poll,
            poll_interval_secs: None,
            enabled: true,
            last_error: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        service.watch(&dir).await.unwrap();

        // Verify it's being watched.
        assert!(service
            .inner
            .lock()
            .await
            .watched_paths
            .contains(tmp.path()));

        // Shutdown and verify everything is cleaned up.
        service.shutdown().await;

        let inner = service.inner.lock().await;
        let paths_empty = inner.watched_paths.is_empty();
        let native_dropped = inner.native_debouncer.is_none();
        let poll_dropped = inner.poll_debouncer.is_none();
        drop(inner);

        assert!(paths_empty);
        assert!(native_dropped);
        assert!(poll_dropped);
    }

    #[tokio::test]
    async fn watch_after_shutdown_returns_error() {
        let config = WatcherRuntimeConfig {
            debounce_ms: 500,
            default_poll_interval_secs: 10,
        };
        let service = WatcherService::new(config).unwrap();
        service.shutdown().await;

        let tmp = tempfile::tempdir().unwrap();
        let dir = archivis_core::models::WatchedDirectory {
            id: uuid::Uuid::new_v4(),
            path: tmp.path().to_string_lossy().to_string(),
            watch_mode: WatchMode::Poll,
            poll_interval_secs: None,
            enabled: true,
            last_error: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let result = service.watch(&dir).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("shut down"));
    }
}
