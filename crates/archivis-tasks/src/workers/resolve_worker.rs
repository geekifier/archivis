use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use archivis_core::errors::TaskError;
use archivis_core::models::TaskType;
use archivis_db::MetadataRuleRepository;
use archivis_storage::StorageBackend;
use uuid::Uuid;

use crate::queue::{ProgressSender, Worker};
use crate::resolve::ResolutionService;

/// Worker that resolves a book (or batch of books) via metadata providers.
pub struct ResolveWorker<S: StorageBackend> {
    service: Arc<ResolutionService<S>>,
}

impl<S: StorageBackend> ResolveWorker<S> {
    pub fn new(service: Arc<ResolutionService<S>>) -> Self {
        Self { service }
    }

    async fn execute_batch(
        &self,
        book_ids_val: &serde_json::Value,
        manual_refresh: bool,
        metadata_rules: &[archivis_core::models::MetadataRule],
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
        let mut resolved = 0_usize;
        for (index, id_str) in book_ids.iter().enumerate() {
            let book_id = Uuid::parse_str(id_str)
                .map_err(|e| TaskError::Failed(format!("invalid UUID '{id_str}': {e}")))?;

            #[allow(clippy::cast_possible_truncation)]
            let pct = if total > 0 {
                ((index * 100) / total) as u8
            } else {
                0
            };

            progress
                .send_progress(pct, Some(format!("Resolving book {}/{}", index + 1, total)))
                .await;

            match self
                .service
                .resolve_queued_book(book_id, manual_refresh, metadata_rules)
                .await
            {
                Ok(Some(outcome)) => {
                    resolved += 1;
                    results.push(serde_json::json!({
                        "book_id": book_id.to_string(),
                        "candidates": outcome.resolver_result.candidates.len(),
                        "auto_applied": outcome.auto_applied,
                        "best_tier": outcome.best_tier.map(|tier| tier.to_string()),
                        "decision_reason": outcome.decision_reason,
                    }));
                }
                Ok(None) => {
                    results.push(serde_json::json!({
                        "book_id": book_id.to_string(),
                        "noop": true,
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

        progress
            .send_progress(100, Some("Batch resolution complete".into()))
            .await;

        Ok(serde_json::json!({
            "mode": "batch",
            "total": total,
            "resolved": resolved,
            "identified": resolved,
            "failed": errors.len(),
            "results": results,
            "errors": errors,
        }))
    }

    async fn execute_single(
        &self,
        payload: &serde_json::Value,
        manual_refresh: bool,
        metadata_rules: &[archivis_core::models::MetadataRule],
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
            .send_progress(0, Some(format!("Resolving book {book_id}")))
            .await;

        let Some(outcome) = self
            .service
            .resolve_queued_book(book_id, manual_refresh, metadata_rules)
            .await?
        else {
            progress
                .send_progress(100, Some("Resolution skipped".into()))
                .await;

            return Ok(serde_json::json!({
                "mode": "single",
                "book_id": book_id.to_string(),
                "noop": true,
            }));
        };

        progress
            .send_progress(100, Some("Resolution complete".into()))
            .await;

        Ok(serde_json::json!({
            "mode": "single",
            "book_id": book_id.to_string(),
            "candidates": outcome.resolver_result.candidates.len(),
            "auto_applied": outcome.auto_applied,
            "best_score": outcome.resolver_result.best_match.as_ref().map(|best| best.score),
            "best_tier": outcome.best_tier.map(|tier| tier.to_string()),
            "decision_reason": outcome.decision_reason,
        }))
    }
}

impl<S: StorageBackend + 'static> Worker for ResolveWorker<S> {
    fn task_type(&self) -> TaskType {
        TaskType::ResolveBook
    }

    fn execute(
        &self,
        payload: serde_json::Value,
        progress: ProgressSender,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, TaskError>> + Send + '_>> {
        Box::pin(async move {
            let manual_refresh = payload
                .get("manual_refresh")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);

            // Load metadata rules once per task execution.
            let metadata_rules: Vec<archivis_core::models::MetadataRule> =
                MetadataRuleRepository::list_enabled(self.service.db_pool())
                    .await
                    .unwrap_or_default();

            if let Some(book_ids_val) = payload.get("book_ids") {
                self.execute_batch(book_ids_val, manual_refresh, &metadata_rules, &progress)
                    .await
            } else {
                self.execute_single(&payload, manual_refresh, &metadata_rules, &progress)
                    .await
            }
        })
    }
}
