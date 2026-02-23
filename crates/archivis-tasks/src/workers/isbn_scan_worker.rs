use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use archivis_core::errors::TaskError;
use archivis_core::models::TaskType;
use archivis_storage::StorageBackend;
use uuid::Uuid;

use crate::isbn_scan::IsbnScanService;
use crate::queue::{ProgressSender, Worker};

/// Worker that scans book file content for ISBNs.
pub struct IsbnScanWorker<S: StorageBackend> {
    service: Arc<IsbnScanService<S>>,
}

impl<S: StorageBackend> IsbnScanWorker<S> {
    pub fn new(service: Arc<IsbnScanService<S>>) -> Self {
        Self { service }
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
            // Support both single book and batch modes:
            // Single: { "book_id": "uuid" }
            // Batch:  { "book_ids": ["uuid1", "uuid2", ...] }

            if let Some(book_ids_val) = payload.get("book_ids") {
                // Batch mode
                let book_ids: Vec<String> = book_ids_val
                    .as_array()
                    .ok_or_else(|| TaskError::Failed("'book_ids' must be an array".into()))?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .ok_or_else(|| {
                                TaskError::Failed("book_ids elements must be strings".into())
                            })
                            .map(String::from)
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let total = book_ids.len();
                let mut results = Vec::new();
                let mut errors = Vec::new();

                for (i, id_str) in book_ids.iter().enumerate() {
                    let book_id = Uuid::parse_str(id_str)
                        .map_err(|e| TaskError::Failed(format!("invalid UUID '{id_str}': {e}")))?;

                    #[allow(clippy::cast_possible_truncation)]
                    let pct = if total > 0 {
                        ((i * 100) / total) as u8
                    } else {
                        0
                    };

                    progress
                        .send_progress(
                            pct,
                            Some(format!("Scanning book {}/{total} for ISBNs", i + 1)),
                        )
                        .await;

                    match self.service.scan_book(book_id).await {
                        Ok(result) => {
                            results.push(serde_json::json!({
                                "book_id": book_id.to_string(),
                                "isbns_found": result.isbns_found,
                                "isbns_stored": result.isbns_stored,
                                "files_scanned": result.files_scanned,
                            }));
                        }
                        Err(e) => {
                            errors.push(serde_json::json!({
                                "book_id": book_id.to_string(),
                                "error": e.to_string(),
                            }));
                        }
                    }
                }

                progress
                    .send_progress(100, Some("Batch ISBN scan complete".into()))
                    .await;

                Ok(serde_json::json!({
                    "mode": "batch",
                    "total": total,
                    "scanned": results.len(),
                    "failed": errors.len(),
                    "results": results,
                    "errors": errors,
                }))
            } else {
                // Single book mode
                let book_id_str =
                    payload
                        .get("book_id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            TaskError::Failed("missing 'book_id' or 'book_ids' in payload".into())
                        })?;

                let book_id = Uuid::parse_str(book_id_str)
                    .map_err(|e| TaskError::Failed(format!("invalid UUID '{book_id_str}': {e}")))?;

                progress
                    .send_progress(0, Some(format!("Scanning book {book_id} for ISBNs")))
                    .await;

                let result = self.service.scan_book(book_id).await?;

                progress
                    .send_progress(100, Some("ISBN scan complete".into()))
                    .await;

                Ok(serde_json::json!({
                    "mode": "single",
                    "book_id": book_id.to_string(),
                    "isbns_found": result.isbns_found,
                    "isbns_stored": result.isbns_stored,
                    "files_scanned": result.files_scanned,
                }))
            }
        })
    }
}
