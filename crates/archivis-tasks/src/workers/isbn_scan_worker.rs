use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use archivis_core::errors::TaskError;
use archivis_core::models::TaskType;
use archivis_storage::StorageBackend;
use uuid::Uuid;

use crate::isbn_scan::IsbnScanService;
use crate::queue::{ProgressSender, TaskQueue, Worker};

/// Worker that scans book file content for ISBNs.
pub struct IsbnScanWorker<S: StorageBackend> {
    service: Arc<IsbnScanService<S>>,
    task_queue: Option<Arc<TaskQueue>>,
}

impl<S: StorageBackend> IsbnScanWorker<S> {
    pub fn new(service: Arc<IsbnScanService<S>>) -> Self {
        Self {
            service,
            task_queue: None,
        }
    }

    #[must_use]
    pub fn with_resolution_queue(mut self, task_queue: Arc<TaskQueue>) -> Self {
        self.task_queue = Some(task_queue);
        self
    }

    async fn execute_batch(
        &self,
        book_ids_val: &serde_json::Value,
        resolve_after_scan: bool,
        progress: &ProgressSender,
    ) -> Result<serde_json::Value, TaskError> {
        let book_ids: Vec<String> = book_ids_val
            .as_array()
            .ok_or_else(|| TaskError::Failed("'book_ids' must be an array".into()))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| TaskError::Failed("book_ids elements must be strings".into()))
                    .map(String::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let total = book_ids.len();
        let mut results = Vec::new();
        let mut errors = Vec::new();
        let mut resolve_book_ids = Vec::new();

        for (index, id_str) in book_ids.iter().enumerate() {
            // Check for cancellation before each book
            if progress.is_cancelled() {
                return Err(TaskError::Cancelled);
            }

            let book_id = Uuid::parse_str(id_str)
                .map_err(|e| TaskError::Failed(format!("invalid UUID '{id_str}': {e}")))?;

            #[allow(clippy::cast_possible_truncation)]
            let pct = if total > 0 {
                ((index * 100) / total) as u8
            } else {
                0
            };

            progress
                .send_progress_with_data(
                    pct,
                    Some(format!("Scanning book {}/{total} for ISBNs", index + 1)),
                    Some(serde_json::json!({
                        "processed": index,
                        "total": total,
                        "scanned": results.len(),
                        "failed": errors.len(),
                    })),
                )
                .await;

            match self.service.scan_book(book_id).await {
                Ok(result) => {
                    if resolve_after_scan || result.isbns_stored > 0 {
                        resolve_book_ids.push(book_id.to_string());
                    }
                    results.push(serde_json::json!({
                        "book_id": book_id.to_string(),
                        "isbns_found": result.isbns_found,
                        "isbns_stored": result.isbns_stored,
                        "files_scanned": result.files_scanned,
                        "lccns_found": result.lccns_found,
                        "lccns_stored": result.lccns_stored,
                    }));
                }
                Err(error) => {
                    errors.push(serde_json::json!({
                        "book_id": book_id.to_string(),
                        "error": error.to_string(),
                    }));
                }
            }
        }

        let resolution_parent = progress.resolution_parent();
        enqueue_resolution_batch(
            self.task_queue.as_ref(),
            resolution_parent,
            resolve_book_ids,
        )
        .await;

        progress
            .send_progress_with_data(
                100,
                Some("Batch ISBN scan complete".into()),
                Some(serde_json::json!({
                    "processed": total,
                    "total": total,
                    "scanned": results.len(),
                    "failed": errors.len(),
                })),
            )
            .await;

        Ok(serde_json::json!({
            "mode": "batch",
            "total": total,
            "scanned": results.len(),
            "failed": errors.len(),
            "results": results,
            "errors": errors,
        }))
    }

    async fn execute_single(
        &self,
        payload: &serde_json::Value,
        resolve_after_scan: bool,
        progress: &ProgressSender,
    ) -> Result<serde_json::Value, TaskError> {
        let book_id_str = payload
            .get("book_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                TaskError::Failed("missing 'book_id' or 'book_ids' in payload".into())
            })?;

        let book_id = Uuid::parse_str(book_id_str)
            .map_err(|e| TaskError::Failed(format!("invalid UUID '{book_id_str}': {e}")))?;

        progress
            .send_progress(0, Some(format!("Scanning book {book_id} for ISBNs")))
            .await;

        let result = self.service.scan_book(book_id).await?;

        if resolve_after_scan || result.isbns_stored > 0 {
            let resolution_parent = progress.resolution_parent();
            enqueue_resolution_single(self.task_queue.as_ref(), resolution_parent, book_id).await;
        }

        progress
            .send_progress(100, Some("ISBN scan complete".into()))
            .await;

        Ok(serde_json::json!({
            "mode": "single",
            "book_id": book_id.to_string(),
            "isbns_found": result.isbns_found,
            "isbns_stored": result.isbns_stored,
            "files_scanned": result.files_scanned,
            "lccns_found": result.lccns_found,
            "lccns_stored": result.lccns_stored,
        }))
    }
}

impl<S: StorageBackend + 'static> Worker for IsbnScanWorker<S> {
    fn task_type(&self) -> TaskType {
        TaskType::ScanIsbn
    }

    fn execute(
        &self,
        payload: serde_json::Value,
        progress: ProgressSender,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, TaskError>> + Send + '_>> {
        Box::pin(async move {
            let resolve_after_scan = payload
                .get("resolve_after_scan")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);

            if let Some(book_ids_val) = payload.get("book_ids") {
                self.execute_batch(book_ids_val, resolve_after_scan, &progress)
                    .await
            } else {
                self.execute_single(&payload, resolve_after_scan, &progress)
                    .await
            }
        })
    }
}

async fn enqueue_resolution_single(
    task_queue: Option<&Arc<TaskQueue>>,
    parent_task_id: Uuid,
    book_id: Uuid,
) {
    let Some(task_queue) = task_queue else {
        tracing::warn!(%book_id, "resolution enqueue requested after ISBN scan without task queue");
        return;
    };

    let result = task_queue
        .enqueue_child(
            TaskType::ResolveBook,
            serde_json::json!({
                "book_id": book_id.to_string(),
            }),
            parent_task_id,
        )
        .await;

    log_enqueue_result(result, Some(book_id), None);
}

async fn enqueue_resolution_batch(
    task_queue: Option<&Arc<TaskQueue>>,
    parent_task_id: Uuid,
    book_ids: Vec<String>,
) {
    if book_ids.is_empty() {
        return;
    }

    let Some(task_queue) = task_queue else {
        tracing::warn!(
            count = book_ids.len(),
            "resolution enqueue requested after ISBN scan without task queue"
        );
        return;
    };

    let count = book_ids.len();
    let result = task_queue
        .enqueue_child(
            TaskType::ResolveBook,
            serde_json::json!({
                "book_ids": book_ids,
            }),
            parent_task_id,
        )
        .await;

    log_enqueue_result(result, None, Some(count));
}

fn log_enqueue_result(
    result: Result<Uuid, TaskError>,
    book_id: Option<Uuid>,
    count: Option<usize>,
) {
    match result {
        Ok(task_id) => {
            tracing::debug!(
                %task_id,
                ?book_id,
                ?count,
                "enqueued resolution after ISBN scan"
            );
        }
        Err(error) => {
            tracing::warn!(
                ?book_id,
                ?count,
                %error,
                "failed to enqueue resolution after ISBN scan"
            );
        }
    }
}
