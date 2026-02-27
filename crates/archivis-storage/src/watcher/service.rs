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
    /// Debouncer for paths using native OS events.
    native_debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
    /// Debouncer for paths using polling.
    poll_debouncer: Debouncer<notify::PollWatcher, NoCache>,
    /// Set of currently-watched paths and their assigned backend.
    watched_paths: HashSet<PathBuf>,
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
                native_debouncer,
                poll_debouncer,
                watched_paths: HashSet::new(),
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
                inner
                    .native_debouncer
                    .watch(&path, RecursiveMode::Recursive)
                    .map_err(|e| {
                        StorageError::Watcher(format!(
                            "failed to watch {} with native events: {e}",
                            path.display()
                        ))
                    })?;
                info!(path = %directory.path, "watching with native events");
            }
            WatchMode::Poll => {
                let interval = directory
                    .poll_interval_secs
                    .map_or(inner.config.default_poll_interval_secs, i64::unsigned_abs);
                // Note: notify's PollWatcher uses a single global poll interval
                // configured at creation time. Per-directory intervals would require
                // separate PollWatcher instances (future enhancement).
                inner
                    .poll_debouncer
                    .watch(&path, RecursiveMode::Recursive)
                    .map_err(|e| {
                        StorageError::Watcher(format!(
                            "failed to watch {} with polling: {e}",
                            path.display()
                        ))
                    })?;
                info!(
                    path = %directory.path,
                    interval_secs = interval,
                    "watching with polling"
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
        let native_result = inner.native_debouncer.unwatch(path);
        let poll_result = inner.poll_debouncer.unwatch(path);
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

    /// Graceful shutdown — stop all watchers and close the event channel.
    pub async fn shutdown(&self) {
        let mut inner = self.inner.lock().await;

        // Collect paths first to avoid borrowing `inner` mutably twice.
        let paths: Vec<PathBuf> = inner.watched_paths.drain().collect();
        for path in &paths {
            let _ = inner.native_debouncer.unwatch(path);
            let _ = inner.poll_debouncer.unwatch(path);
        }
        drop(inner);

        info!("watcher service shut down");
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
                continue;
            }
            WatcherEvent::FileChanged { path: path.clone() }
        } else if event.kind.is_remove() {
            // For removals, still apply extension filter but not temp file filter
            // (the file is gone, we can't check further).
            if !has_supported_extension(path) {
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
}
