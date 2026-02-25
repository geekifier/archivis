use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use archivis_core::errors::TaskError;
use archivis_core::models::TaskType;
use archivis_storage::StorageBackend;

use crate::import::{
    BulkImportResult, BulkImportService, FileOutcome, ImportProgress, ImportService,
};
use crate::queue::{ProgressSender, TaskQueue, Worker};

// ---------------------------------------------------------------------------
// ImportFileWorker
// ---------------------------------------------------------------------------

/// Worker that imports a single ebook file via [`ImportService`].
pub struct ImportFileWorker<S: StorageBackend> {
    import_service: Arc<ImportService<S>>,
    task_queue: Option<Arc<TaskQueue>>,
    isbn_scan_on_import: bool,
}

impl<S: StorageBackend> ImportFileWorker<S> {
    pub fn new(import_service: Arc<ImportService<S>>) -> Self {
        Self {
            import_service,
            task_queue: None,
            isbn_scan_on_import: false,
        }
    }

    /// Enable automatic ISBN scanning after successful imports.
    #[must_use]
    pub fn with_isbn_scan(mut self, task_queue: Arc<TaskQueue>, enabled: bool) -> Self {
        self.task_queue = Some(task_queue);
        self.isbn_scan_on_import = enabled;
        self
    }
}

impl<S: StorageBackend + 'static> Worker for ImportFileWorker<S> {
    fn task_type(&self) -> TaskType {
        TaskType::ImportFile
    }

    fn execute(
        &self,
        payload: serde_json::Value,
        progress: ProgressSender,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, TaskError>> + Send + '_>> {
        Box::pin(async move {
            let file_path: PathBuf = payload
                .get("file_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| TaskError::Failed("missing 'file_path' in payload".into()))?
                .into();

            progress
                .send_progress(0, Some(format!("Importing: {}", file_path.display())))
                .await;

            let result = self
                .import_service
                .import_file(&file_path)
                .await
                .map_err(|e| TaskError::Failed(e.to_string()))?;

            progress
                .send_progress(100, Some("Import complete".into()))
                .await;

            // Enqueue ISBN content scan as a child task if enabled
            if self.isbn_scan_on_import {
                if let Some(queue) = &self.task_queue {
                    let scan_payload = serde_json::json!({
                        "book_id": result.book_id.to_string(),
                    });
                    match queue
                        .enqueue_child(TaskType::ScanIsbn, scan_payload, progress.task_id())
                        .await
                    {
                        Ok(task_id) => {
                            tracing::debug!(
                                %task_id,
                                book_id = %result.book_id,
                                "enqueued ISBN scan as child of import",
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                book_id = %result.book_id,
                                error = %e,
                                "failed to enqueue ISBN scan after import",
                            );
                        }
                    }
                }
            }

            let json = serde_json::json!({
                "book_id": result.book_id.to_string(),
                "book_file_id": result.book_file_id.to_string(),
                "status": format!("{}", result.status),
                "confidence": result.confidence,
                "cover_extracted": result.cover_extracted,
            });

            Ok(json)
        })
    }
}

// ---------------------------------------------------------------------------
// ImportDirectoryWorker
// ---------------------------------------------------------------------------

/// Worker that bulk-imports a directory of ebook files via [`BulkImportService`].
pub struct ImportDirectoryWorker<S: StorageBackend> {
    bulk_import_service: Arc<BulkImportService<S>>,
    task_queue: Option<Arc<TaskQueue>>,
    isbn_scan_on_import: bool,
}

impl<S: StorageBackend> ImportDirectoryWorker<S> {
    pub fn new(bulk_import_service: Arc<BulkImportService<S>>) -> Self {
        Self {
            bulk_import_service,
            task_queue: None,
            isbn_scan_on_import: false,
        }
    }

    /// Enable automatic ISBN scanning after successful bulk imports.
    #[must_use]
    pub fn with_isbn_scan(mut self, task_queue: Arc<TaskQueue>, enabled: bool) -> Self {
        self.task_queue = Some(task_queue);
        self.isbn_scan_on_import = enabled;
        self
    }
}

impl<S: StorageBackend + 'static> Worker for ImportDirectoryWorker<S> {
    fn task_type(&self) -> TaskType {
        TaskType::ImportDirectory
    }

    fn execute(
        &self,
        payload: serde_json::Value,
        progress: ProgressSender,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, TaskError>> + Send + '_>> {
        Box::pin(async move {
            let dir_path: PathBuf = payload
                .get("directory_path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| TaskError::Failed("missing 'directory_path' in payload".into()))?
                .into();

            progress
                .send_progress(0, Some(format!("Scanning: {}", dir_path.display())))
                .await;

            let adapter = BroadcastProgress::new(progress.clone());

            let result = self
                .bulk_import_service
                .import_directory(&dir_path, &adapter)
                .await
                .map_err(|e| TaskError::Failed(e.to_string()))?;

            // Check if we were cancelled mid-import
            if progress.is_cancelled() {
                return Err(TaskError::Cancelled);
            }

            progress
                .send_progress(100, Some("Directory import complete".into()))
                .await;

            // Enqueue batch ISBN content scan as a child task for all successfully imported books
            if self.isbn_scan_on_import && !result.imported.is_empty() {
                if let Some(queue) = &self.task_queue {
                    let book_ids: Vec<String> = result
                        .imported
                        .iter()
                        .map(|r| r.book_id.to_string())
                        .collect();
                    let scan_payload = serde_json::json!({
                        "book_ids": book_ids,
                    });
                    match queue
                        .enqueue_child(TaskType::ScanIsbn, scan_payload, progress.task_id())
                        .await
                    {
                        Ok(task_id) => {
                            tracing::debug!(
                                %task_id,
                                count = book_ids.len(),
                                "enqueued batch ISBN scan as child of directory import",
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                count = result.imported.len(),
                                error = %e,
                                "failed to enqueue batch ISBN scan after directory import",
                            );
                        }
                    }
                }
            }

            let json = serde_json::json!({
                "imported": result.imported.len(),
                "skipped": result.skipped.len(),
                "failed": result.failed.len(),
                "total": result.imported.len() + result.skipped.len() + result.failed.len(),
            });

            Ok(json)
        })
    }
}

// ---------------------------------------------------------------------------
// BroadcastProgress adapter
// ---------------------------------------------------------------------------

/// Bridges the synchronous [`ImportProgress`] callbacks to the async
/// [`ProgressSender`] by spawning fire-and-forget tasks for each update.
struct BroadcastProgress {
    sender: ProgressSender,
    total_files: AtomicUsize,
    imported: AtomicUsize,
    skipped: AtomicUsize,
    failed: AtomicUsize,
}

impl BroadcastProgress {
    fn new(sender: ProgressSender) -> Self {
        Self {
            sender,
            total_files: AtomicUsize::new(0),
            imported: AtomicUsize::new(0),
            skipped: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
        }
    }
}

impl ImportProgress for BroadcastProgress {
    fn on_import_start(&self, total_files: usize) {
        self.total_files.store(total_files, Ordering::SeqCst);
    }

    fn on_file_start(&self, index: usize, path: &std::path::Path) {
        let total = self.total_files.load(Ordering::SeqCst);
        if total == 0 {
            return;
        }
        #[allow(clippy::cast_possible_truncation)]
        let progress = ((index * 100) / total) as u8;
        let message = Some(format!(
            "Processing: {}",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        let sender = self.sender.clone();
        tokio::spawn(async move {
            sender.send_progress(progress, message).await;
        });
    }

    fn on_file_complete(&self, index: usize, _path: &std::path::Path, outcome: &FileOutcome) {
        // Update counters
        match outcome {
            FileOutcome::Imported(_) => {
                self.imported.fetch_add(1, Ordering::SeqCst);
            }
            FileOutcome::Skipped(_) => {
                self.skipped.fetch_add(1, Ordering::SeqCst);
            }
            FileOutcome::Failed(_) => {
                self.failed.fetch_add(1, Ordering::SeqCst);
            }
        }

        let total = self.total_files.load(Ordering::SeqCst);
        if total == 0 {
            return;
        }
        let processed = index + 1;
        #[allow(clippy::cast_possible_truncation)]
        let progress = ((processed * 100) / total) as u8;
        let message = Some(format!("{processed}/{total} files processed"));

        // Structured progress data
        let data = Some(serde_json::json!({
            "processed": processed,
            "total": total,
            "imported": self.imported.load(Ordering::SeqCst),
            "skipped": self.skipped.load(Ordering::SeqCst),
            "failed": self.failed.load(Ordering::SeqCst),
        }));

        let sender = self.sender.clone();
        tokio::spawn(async move {
            sender
                .send_progress_with_data(progress, message, data)
                .await;
        });
    }

    fn on_import_complete(&self, result: &BulkImportResult) {
        let message = Some(format!(
            "Complete: {} imported, {} skipped, {} failed",
            result.imported.len(),
            result.skipped.len(),
            result.failed.len(),
        ));
        let sender = self.sender.clone();
        tokio::spawn(async move {
            sender.send_progress(100, message).await;
        });
    }

    fn should_cancel(&self) -> bool {
        self.sender.is_cancelled()
    }
}
