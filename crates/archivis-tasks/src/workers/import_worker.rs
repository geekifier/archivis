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
use crate::queue::{ProgressSender, Worker};

// ---------------------------------------------------------------------------
// ImportFileWorker
// ---------------------------------------------------------------------------

/// Worker that imports a single ebook file via [`ImportService`].
pub struct ImportFileWorker<S: StorageBackend> {
    import_service: Arc<ImportService<S>>,
}

impl<S: StorageBackend> ImportFileWorker<S> {
    pub fn new(import_service: Arc<ImportService<S>>) -> Self {
        Self { import_service }
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
}

impl<S: StorageBackend> ImportDirectoryWorker<S> {
    pub fn new(bulk_import_service: Arc<BulkImportService<S>>) -> Self {
        Self {
            bulk_import_service,
        }
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

            progress
                .send_progress(100, Some("Directory import complete".into()))
                .await;

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
}

impl BroadcastProgress {
    fn new(sender: ProgressSender) -> Self {
        Self {
            sender,
            total_files: AtomicUsize::new(0),
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

    fn on_file_complete(&self, index: usize, _path: &std::path::Path, _outcome: &FileOutcome) {
        let total = self.total_files.load(Ordering::SeqCst);
        if total == 0 {
            return;
        }
        #[allow(clippy::cast_possible_truncation)]
        let progress = (((index + 1) * 100) / total) as u8;
        let message = Some(format!("{}/{} files processed", index + 1, total));
        let sender = self.sender.clone();
        tokio::spawn(async move {
            sender.send_progress(progress, message).await;
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
}
