//! Integration tests for the filesystem watcher service.
//!
//! These tests exercise `WatcherService` with real filesystem operations --
//! creating, modifying, renaming, and deleting files -- and verify that the
//! correct `WatcherEvent`s are emitted (or not) after debouncing and filtering.
//!
//! Test directories are ephemeral (`tempfile::TempDir`). Debounce is set short
//! (200ms) and poll interval short (2s) for fast feedback. Timeouts are generous
//! (5s+) to avoid flakiness on slow CI runners.
//!
//! # macOS path canonicalization
//!
//! On macOS, `/var` is a symlink to `/private/var`. `FSEvents` reports canonical
//! (resolved) paths, so we must canonicalize `TempDir` paths before comparison.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use archivis_core::models::{WatchMode, WatchedDirectory};
use archivis_storage::watcher::{WatcherEvent, WatcherRuntimeConfig, WatcherService};
use chrono::Utc;
use tokio::sync::mpsc;
use uuid::Uuid;

// ── Constants ─────────────────────────────────────────────────────────

/// Short debounce for test speed (200ms).
const TEST_DEBOUNCE_MS: u64 = 200;

/// Short poll interval for polling-mode tests (2s).
const TEST_POLL_INTERVAL_SECS: u64 = 2;

/// Generous timeout to wait for an event before declaring failure.
const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

/// Longer timeout for polling mode (poll interval + debounce + buffer).
const POLL_EVENT_TIMEOUT: Duration = Duration::from_secs(8);

/// Time to wait when asserting that *no* event arrives.
const NO_EVENT_TIMEOUT: Duration = Duration::from_millis(1500);

/// Small pause after filesystem operations to let the OS flush events.
const FS_SETTLE: Duration = Duration::from_millis(100);

// ── Helpers ───────────────────────────────────────────────────────────

/// Canonicalize a path, resolving symlinks. On macOS this resolves
/// `/var` -> `/private/var` to match FSEvents-reported paths.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Wait for the next `WatcherEvent` on `rx`, returning `None` on timeout.
async fn wait_for_event(
    rx: &mut mpsc::Receiver<WatcherEvent>,
    timeout: Duration,
) -> Option<WatcherEvent> {
    tokio::time::timeout(timeout, rx.recv())
        .await
        .ok()
        .flatten()
}

/// Assert that no event arrives within `timeout`. Panics if one does.
async fn assert_no_event(rx: &mut mpsc::Receiver<WatcherEvent>, timeout: Duration) {
    match tokio::time::timeout(timeout, rx.recv()).await {
        Ok(Some(event)) => panic!("expected no event, but received: {event:?}"),
        Ok(None) => panic!("channel closed unexpectedly"),
        Err(_) => { /* timeout elapsed -- correct, no event received */ }
    }
}

/// Build a `WatchedDirectory` for the given path with native watch mode.
fn watched_dir_native(path: &Path) -> WatchedDirectory {
    WatchedDirectory {
        id: Uuid::new_v4(),
        path: path.to_string_lossy().into_owned(),
        watch_mode: WatchMode::Native,
        poll_interval_secs: None,
        enabled: true,
        last_error: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// Build a `WatchedDirectory` for the given path with poll watch mode.
fn watched_dir_poll(path: &Path) -> WatchedDirectory {
    WatchedDirectory {
        id: Uuid::new_v4(),
        path: path.to_string_lossy().into_owned(),
        watch_mode: WatchMode::Poll,
        poll_interval_secs: Some(2),
        enabled: true,
        last_error: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// Create a `WatcherService` with short debounce, watch the given directory
/// with native mode, and return the service, event receiver, and the
/// canonicalized watch directory path.
async fn setup_native_watcher(
    dir: &Path,
) -> (WatcherService, mpsc::Receiver<WatcherEvent>, PathBuf) {
    let canon_dir = canonical(dir);
    let config = WatcherRuntimeConfig {
        debounce_ms: TEST_DEBOUNCE_MS,
        default_poll_interval_secs: TEST_POLL_INTERVAL_SECS,
    };
    let service = WatcherService::new(config).expect("failed to create WatcherService");
    let mut rx = service
        .event_receiver()
        .await
        .expect("failed to take event receiver");

    let wd = watched_dir_native(&canon_dir);
    service.watch(&wd).await.expect("failed to watch directory");

    // Drain any spurious initial events (some backends emit events on watch start).
    drain_initial_events(&mut rx).await;

    (service, rx, canon_dir)
}

/// Drain any events that arrive shortly after watch setup (some OS backends
/// emit events when the watcher is first attached).
async fn drain_initial_events(rx: &mut mpsc::Receiver<WatcherEvent>) {
    let drain_window = Duration::from_millis(500);
    while tokio::time::timeout(drain_window, rx.recv())
        .await
        .ok()
        .flatten()
        .is_some()
    {}
}

/// Collect `FileChanged` events from the receiver until `expected_count` is
/// reached or `timeout` expires. Returns the collected paths.
async fn collect_file_changed_events(
    rx: &mut mpsc::Receiver<WatcherEvent>,
    timeout: Duration,
    expected_count: usize,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;

    while paths.len() < expected_count {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(WatcherEvent::FileChanged { path })) => {
                paths.push(path);
            }
            Ok(Some(WatcherEvent::Error { .. })) => {
                // Ignore watcher errors in tests (may occur on some platforms).
            }
            Ok(Some(_) | None) | Err(_) => break,
        }
    }
    paths
}

// ── Test 1: New EPUB file triggers event ──────────────────────────────

#[tokio::test]
async fn new_epub_file_triggers_event() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let (_service, mut rx, canon_dir) = setup_native_watcher(tmp.path()).await;

    // Create a file with .epub extension.
    let epub_path = canon_dir.join("test-book.epub");
    fs::write(&epub_path, b"fake epub content").expect("write epub");

    let event = wait_for_event(&mut rx, EVENT_TIMEOUT)
        .await
        .expect("expected FileChanged event for .epub file");

    match event {
        WatcherEvent::FileChanged { path } => {
            assert_eq!(path, epub_path);
        }
        other => panic!("expected FileChanged, got {other:?}"),
    }
}

// ── Test 2: Non-ebook file ignored ────────────────────────────────────

#[tokio::test]
async fn non_ebook_file_ignored() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let (_service, mut rx, canon_dir) = setup_native_watcher(tmp.path()).await;

    // Create a .jpg file -- should be filtered out.
    let jpg_path = canon_dir.join("cover.jpg");
    fs::write(&jpg_path, b"fake jpg content").expect("write jpg");

    assert_no_event(&mut rx, NO_EVENT_TIMEOUT).await;
}

// ── Test 3: Hidden file ignored ───────────────────────────────────────

#[tokio::test]
async fn hidden_file_ignored() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let (_service, mut rx, canon_dir) = setup_native_watcher(tmp.path()).await;

    // Create a hidden .epub file -- should be filtered out.
    let hidden_path = canon_dir.join(".hidden-book.epub");
    fs::write(&hidden_path, b"fake epub content").expect("write hidden epub");

    assert_no_event(&mut rx, NO_EVENT_TIMEOUT).await;
}

// ── Test 4: Temporary file ignored, rename triggers event ─────────────

#[tokio::test]
async fn temp_file_ignored_rename_triggers_event() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let (_service, mut rx, canon_dir) = setup_native_watcher(tmp.path()).await;

    // Create a .part file (download in progress) -- should be filtered out.
    let part_path = canon_dir.join("book.epub.part");
    fs::write(&part_path, b"partial download content").expect("write .part file");

    // Give debounce time to confirm no event for .part.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Rename to .epub -- this should trigger an event.
    let epub_path = canon_dir.join("book.epub");
    fs::rename(&part_path, &epub_path).expect("rename .part to .epub");

    let event = wait_for_event(&mut rx, EVENT_TIMEOUT)
        .await
        .expect("expected FileChanged event after rename to .epub");

    match event {
        WatcherEvent::FileChanged { path } => {
            assert_eq!(path, epub_path);
        }
        other => panic!("expected FileChanged for book.epub, got {other:?}"),
    }
}

// ── Test 5: File modification triggers event ──────────────────────────

#[tokio::test]
async fn file_modification_triggers_event() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let (_service, mut rx, canon_dir) = setup_native_watcher(tmp.path()).await;

    // Create initial file.
    let epub_path = canon_dir.join("modifiable.epub");
    fs::write(&epub_path, b"initial content").expect("write initial");

    // Wait for the initial creation event.
    let event = wait_for_event(&mut rx, EVENT_TIMEOUT)
        .await
        .expect("expected FileChanged for initial creation");
    assert!(matches!(event, WatcherEvent::FileChanged { .. }));

    // Wait for debounce to settle before modifying.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Overwrite the file with different content.
    fs::write(&epub_path, b"modified content -- different bytes").expect("overwrite file");

    let event = wait_for_event(&mut rx, EVENT_TIMEOUT)
        .await
        .expect("expected FileChanged for modification");

    match event {
        WatcherEvent::FileChanged { path } => {
            assert_eq!(path, epub_path);
        }
        other => panic!("expected FileChanged for modification, got {other:?}"),
    }
}

// ── Test 6: File deletion triggers event ──────────────────────────────

#[tokio::test]
async fn file_deletion_triggers_event() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let (_service, mut rx, canon_dir) = setup_native_watcher(tmp.path()).await;

    // Create a file first.
    let epub_path = canon_dir.join("deletable.epub");
    fs::write(&epub_path, b"content to delete").expect("write file");

    // Wait for the creation event.
    let event = wait_for_event(&mut rx, EVENT_TIMEOUT)
        .await
        .expect("expected FileChanged for creation");
    assert!(matches!(event, WatcherEvent::FileChanged { .. }));

    // Wait for debounce to settle.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Delete the file.
    fs::remove_file(&epub_path).expect("delete file");

    // Wait for a deletion-related event. On some platforms (macOS `FSEvents`),
    // the debouncer may emit `FileChanged` instead of `FileRemoved` for
    // deletions, or may not emit any event at all for removed files. We accept
    // either `FileRemoved` (ideal) or `FileChanged` (platform quirk) for the
    // correct path.
    let event = wait_for_event(&mut rx, EVENT_TIMEOUT)
        .await
        .expect("expected an event after file deletion");

    match &event {
        WatcherEvent::FileRemoved { path } | WatcherEvent::FileChanged { path } => {
            assert_eq!(path, &epub_path);
        }
        WatcherEvent::Error { error, .. } => {
            panic!("expected FileRemoved or FileChanged for deleted file, got Error: {error}")
        }
    }
}

// ── Test 7: Rapid creation of multiple files ──────────────────────────

#[tokio::test]
async fn rapid_creation_of_multiple_files() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let (_service, mut rx, canon_dir) = setup_native_watcher(tmp.path()).await;

    let file_count = 10;
    let mut expected_paths: Vec<PathBuf> = Vec::with_capacity(file_count);

    // Create 10 files in quick succession.
    for i in 0..file_count {
        let path = canon_dir.join(format!("book-{i:02}.epub"));
        fs::write(&path, format!("content for book {i}")).expect("write file");
        expected_paths.push(path);
    }

    // Collect events with a generous timeout.
    let timeout = Duration::from_secs(10);
    let mut received_paths = collect_file_changed_events(&mut rx, timeout, file_count).await;

    // Sort both for comparison (event order is not guaranteed).
    expected_paths.sort();
    received_paths.sort();

    assert_eq!(
        received_paths.len(),
        file_count,
        "expected {file_count} FileChanged events, got {} -- paths: {received_paths:?}",
        received_paths.len()
    );
    assert_eq!(received_paths, expected_paths);
}

// ── Test 8: Subdirectory creation ─────────────────────────────────────

#[tokio::test]
async fn subdirectory_file_triggers_event() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let (_service, mut rx, canon_dir) = setup_native_watcher(tmp.path()).await;

    // Create a new subdirectory and place a file inside it.
    let subdir = canon_dir.join("new-subdir");
    fs::create_dir(&subdir).expect("create subdir");

    // Small pause to let the watcher register the new directory.
    tokio::time::sleep(FS_SETTLE).await;

    let epub_path = subdir.join("nested-book.epub");
    fs::write(&epub_path, b"nested epub content").expect("write nested file");

    let event = wait_for_event(&mut rx, EVENT_TIMEOUT)
        .await
        .expect("expected FileChanged for file in subdirectory");

    match event {
        WatcherEvent::FileChanged { path } => {
            assert_eq!(path, epub_path);
        }
        other => panic!("expected FileChanged for nested file, got {other:?}"),
    }
}

// ── Test 9: Polling mode works ────────────────────────────────────────

#[tokio::test]
async fn polling_mode_detects_new_file() {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let canon_dir = canonical(tmp.path());

    let config = WatcherRuntimeConfig {
        debounce_ms: TEST_DEBOUNCE_MS,
        default_poll_interval_secs: TEST_POLL_INTERVAL_SECS,
    };
    let service = WatcherService::new(config).expect("create WatcherService");
    let mut rx = service.event_receiver().await.expect("take event receiver");

    let wd = watched_dir_poll(&canon_dir);
    service.watch(&wd).await.expect("watch with poll mode");

    // Drain any initial events from the polling scan.
    drain_initial_events(&mut rx).await;

    // Create a file -- poll mode will detect it on the next scan cycle.
    let epub_path = canon_dir.join("polled-book.epub");
    fs::write(&epub_path, b"polled epub content").expect("write epub");

    // Use a longer timeout: poll interval + debounce + buffer.
    let event = wait_for_event(&mut rx, POLL_EVENT_TIMEOUT)
        .await
        .expect("expected FileChanged event in poll mode");

    match event {
        WatcherEvent::FileChanged { path } => {
            assert_eq!(path, epub_path);
        }
        other => panic!("expected FileChanged in poll mode, got {other:?}"),
    }
}
