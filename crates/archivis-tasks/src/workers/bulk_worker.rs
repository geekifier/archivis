use std::future::Future;
use std::pin::Pin;

use archivis_core::errors::TaskError;
use archivis_core::models::{
    BulkOperation, BulkTagMode, BulkTaskPayload, BulkUpdateFields, FieldProvenance, MetadataSource,
    TaskType,
};
use archivis_db::{BookFilter, BookRepository, TagRepository};
use uuid::Uuid;

use crate::queue::{ProgressSender, Worker};
use crate::resolve::quality::refresh_quality_score_best_effort;

// ---------------------------------------------------------------------------
// Shared bulk-update logic (used by both the API sync path and the worker)
// ---------------------------------------------------------------------------

/// Error from applying bulk field updates to a single book.
#[derive(Debug)]
pub enum BulkFieldError {
    /// Input failed validation (e.g. unrecognized language).
    Validation(String),
    /// Database or internal error.
    Db(String),
}

impl std::fmt::Display for BulkFieldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(msg) | Self::Db(msg) => f.write_str(msg),
        }
    }
}

/// Apply bulk scalar field updates to a single book.
///
/// Shared between the API sync path and the background bulk worker so both
/// paths have identical semantics:
/// - Rejects unrecognized languages with [`BulkFieldError::Validation`]
/// - Stamps `metadata_provenance` with user provenance on changed fields
/// - Refreshes metadata quality score after changes
///
/// Returns `true` if the book was modified.
pub async fn apply_bulk_update_to_book(
    pool: &archivis_db::DbPool,
    book_id: Uuid,
    fields: &BulkUpdateFields,
) -> Result<bool, BulkFieldError> {
    let mut book = BookRepository::get_by_id(pool, book_id)
        .await
        .map_err(|e| BulkFieldError::Db(format!("book {book_id}: {e}")))?;

    let mut changed = false;

    if let Some(ref language) = fields.language {
        let new_language = validate_bulk_language(language)?;
        if new_language != book.language {
            book.language = new_language;
            book.metadata_provenance.language = Some(bulk_user_provenance());
            changed = true;
        }
    }
    if let Some(rating) = fields.rating {
        if Some(rating) != book.rating {
            book.rating = Some(rating);
            changed = true;
        }
    }
    if let Some(ref pub_id) = fields.publisher_id {
        if *pub_id != book.publisher_id {
            book.publisher_id = *pub_id;
            book.metadata_provenance.publisher = Some(bulk_user_provenance());
            changed = true;
        }
    }

    if changed {
        BookRepository::update(pool, &book)
            .await
            .map_err(|e| BulkFieldError::Db(format!("book {book_id}: {e}")))?;
        refresh_quality_score_best_effort(pool, book_id).await;
    }

    Ok(changed)
}

/// Validate and normalize a language string, matching the API's `validate_language`.
fn validate_bulk_language(input: &str) -> Result<Option<String>, BulkFieldError> {
    if input.is_empty() {
        return Ok(None);
    }
    archivis_core::language::normalize_language(input)
        .map(|code| Some(code.to_string()))
        .ok_or_else(|| BulkFieldError::Validation(format!("unrecognized language: {input:?}")))
}

fn bulk_user_provenance() -> FieldProvenance {
    FieldProvenance {
        origin: MetadataSource::User,
        protected: true,
        applied_candidate_id: None,
    }
}

// ---------------------------------------------------------------------------
// BulkUpdateWorker
// ---------------------------------------------------------------------------

/// Worker that executes bulk update operations on books.
pub struct BulkUpdateWorker {
    db_pool: archivis_db::DbPool,
}

impl BulkUpdateWorker {
    pub fn new(db_pool: archivis_db::DbPool) -> Self {
        Self { db_pool }
    }
}

impl Worker for BulkUpdateWorker {
    fn task_type(&self) -> TaskType {
        TaskType::BulkUpdate
    }

    fn execute(
        &self,
        payload: serde_json::Value,
        progress: ProgressSender,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, TaskError>> + Send + '_>> {
        Box::pin(async move {
            let bulk: BulkTaskPayload = serde_json::from_value(payload)
                .map_err(|e| TaskError::Failed(format!("invalid bulk update payload: {e}")))?;

            let BulkOperation::Update { ref fields } = bulk.operation else {
                return Err(TaskError::Failed(
                    "BulkUpdateWorker received non-update operation".into(),
                ));
            };

            let ids = resolve_scope_ids(&self.db_pool, &bulk).await?;
            execute_bulk_update(&self.db_pool, &ids, fields, &progress).await
        })
    }
}

// ---------------------------------------------------------------------------
// BulkSetTagsWorker
// ---------------------------------------------------------------------------

/// Worker that executes bulk tag operations on books.
pub struct BulkSetTagsWorker {
    db_pool: archivis_db::DbPool,
}

impl BulkSetTagsWorker {
    pub fn new(db_pool: archivis_db::DbPool) -> Self {
        Self { db_pool }
    }
}

impl Worker for BulkSetTagsWorker {
    fn task_type(&self) -> TaskType {
        TaskType::BulkSetTags
    }

    fn execute(
        &self,
        payload: serde_json::Value,
        progress: ProgressSender,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, TaskError>> + Send + '_>> {
        Box::pin(async move {
            let bulk: BulkTaskPayload = serde_json::from_value(payload)
                .map_err(|e| TaskError::Failed(format!("invalid bulk set-tags payload: {e}")))?;

            let (mode, tags) = match bulk.operation {
                BulkOperation::SetTags { mode, ref tags } => (mode, tags.clone()),
                BulkOperation::Update { .. } => {
                    return Err(TaskError::Failed(
                        "BulkSetTagsWorker received non-set-tags operation".into(),
                    ));
                }
            };

            let ids = resolve_scope_ids(&self.db_pool, &bulk).await?;
            let tag_ids: Vec<Uuid> = tags.iter().map(|t| t.tag_id).collect();
            execute_bulk_set_tags(&self.db_pool, &ids, &tag_ids, mode, &progress).await
        })
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Re-execute the filter from the task payload and subtract excluded IDs.
async fn resolve_scope_ids(
    pool: &archivis_db::DbPool,
    bulk: &BulkTaskPayload,
) -> Result<Vec<Uuid>, TaskError> {
    let book_filter = BookFilter::from(&bulk.filter);
    let ids = BookRepository::resolve_scope(pool, &book_filter, &bulk.excluded_ids)
        .await
        .map_err(|e| TaskError::Failed(format!("failed to resolve scope: {e}")))?;

    if ids.is_empty() {
        return Err(TaskError::Failed(
            "scope resolves to zero books after exclusions".into(),
        ));
    }

    Ok(ids)
}

/// Apply scalar field updates to all books, reporting progress.
async fn execute_bulk_update(
    pool: &archivis_db::DbPool,
    ids: &[Uuid],
    fields: &BulkUpdateFields,
    progress: &ProgressSender,
) -> Result<serde_json::Value, TaskError> {
    let total = ids.len();
    let mut updated: u32 = 0;
    let mut failed: u32 = 0;
    let mut errors = Vec::new();

    for (i, &book_id) in ids.iter().enumerate() {
        if progress.is_cancelled() {
            return Err(TaskError::Cancelled);
        }

        match apply_bulk_update_to_book(pool, book_id, fields).await {
            Ok(_) => updated += 1,
            Err(e) => {
                failed += 1;
                errors.push(serde_json::json!({
                    "book_id": book_id.to_string(),
                    "error": e.to_string(),
                }));
            }
        }

        #[allow(clippy::cast_possible_truncation)]
        let pct = if total > 0 {
            (((i + 1) * 100) / total) as u8
        } else {
            100
        };

        if (i + 1) % 50 == 0 || i + 1 == total {
            progress
                .send_progress_with_data(
                    pct,
                    Some(format!("Updated {}/{total} books", i + 1)),
                    Some(serde_json::json!({
                        "processed": i + 1,
                        "total": total,
                        "updated": updated,
                        "failed": failed,
                    })),
                )
                .await;
        }
    }

    Ok(serde_json::json!({
        "total": total,
        "updated": updated,
        "failed": failed,
        "errors": errors,
    }))
}

/// Apply tag changes to all books, reporting progress.
async fn execute_bulk_set_tags(
    pool: &archivis_db::DbPool,
    ids: &[Uuid],
    tag_ids: &[Uuid],
    mode: BulkTagMode,
    progress: &ProgressSender,
) -> Result<serde_json::Value, TaskError> {
    // Verify all tags exist up front.
    for &tag_id in tag_ids {
        TagRepository::get_by_id(pool, tag_id)
            .await
            .map_err(|e| TaskError::Failed(format!("tag {tag_id}: {e}")))?;
    }

    let total = ids.len();
    let mut updated: u32 = 0;
    let mut failed: u32 = 0;
    let mut errors = Vec::new();

    for (i, &book_id) in ids.iter().enumerate() {
        if progress.is_cancelled() {
            return Err(TaskError::Cancelled);
        }

        match apply_tags(pool, book_id, tag_ids, mode).await {
            Ok(()) => updated += 1,
            Err(e) => {
                failed += 1;
                errors.push(serde_json::json!({
                    "book_id": book_id.to_string(),
                    "error": e.to_string(),
                }));
            }
        }

        #[allow(clippy::cast_possible_truncation)]
        let pct = if total > 0 {
            (((i + 1) * 100) / total) as u8
        } else {
            100
        };

        if (i + 1) % 50 == 0 || i + 1 == total {
            progress
                .send_progress_with_data(
                    pct,
                    Some(format!("Tagged {}/{total} books", i + 1)),
                    Some(serde_json::json!({
                        "processed": i + 1,
                        "total": total,
                        "updated": updated,
                        "failed": failed,
                    })),
                )
                .await;
        }
    }

    Ok(serde_json::json!({
        "total": total,
        "updated": updated,
        "failed": failed,
        "errors": errors,
    }))
}

/// Apply tag changes to a single book.
async fn apply_tags(
    pool: &archivis_db::DbPool,
    book_id: Uuid,
    tag_ids: &[Uuid],
    mode: BulkTagMode,
) -> Result<(), TaskError> {
    // Verify book exists
    BookRepository::get_by_id(pool, book_id)
        .await
        .map_err(|e| TaskError::Failed(format!("book {book_id}: {e}")))?;

    let map_err =
        |e: archivis_core::errors::DbError| TaskError::Failed(format!("book {book_id}: {e}"));

    match mode {
        BulkTagMode::Replace => {
            BookRepository::clear_tags(pool, book_id)
                .await
                .map_err(map_err)?;
            for &tag_id in tag_ids {
                BookRepository::add_tag(pool, book_id, tag_id)
                    .await
                    .map_err(map_err)?;
            }
        }
        BulkTagMode::Add => {
            for &tag_id in tag_ids {
                BookRepository::add_tag(pool, book_id, tag_id)
                    .await
                    .map_err(map_err)?;
            }
        }
    }

    Ok(())
}
