use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use archivis_core::models::TaskType;
use archivis_db::BookRepository;

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{BatchIsbnScanRequest, BatchIsbnScanResponse, IsbnScanResponse};

/// POST /api/isbn-scan/book/{id} -- trigger ISBN content scan for a single book.
#[utoipa::path(
    post,
    path = "/api/isbn-scan/book/{id}",
    tag = "isbn-scan",
    params(("id" = Uuid, Path, description = "Book ID")),
    responses(
        (status = 202, description = "ISBN scan task enqueued", body = IsbnScanResponse),
        (status = 404, description = "Book not found"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn scan_book_isbn(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<IsbnScanResponse>), ApiError> {
    // Verify book exists
    BookRepository::get_by_id(state.db_pool(), id).await?;

    let payload = serde_json::json!({
        "book_id": id.to_string(),
    });

    let task_id = state
        .task_queue()
        .enqueue(TaskType::ScanIsbn, payload)
        .await
        .map_err(|e| ApiError::Core(archivis_core::errors::ArchivisError::from(e)))?;

    Ok((StatusCode::ACCEPTED, Json(IsbnScanResponse { task_id })))
}

/// POST /api/isbn-scan/batch -- trigger ISBN content scan for multiple books.
#[utoipa::path(
    post,
    path = "/api/isbn-scan/batch",
    tag = "isbn-scan",
    request_body = BatchIsbnScanRequest,
    responses(
        (status = 202, description = "ISBN scan tasks enqueued", body = BatchIsbnScanResponse),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn batch_scan_isbn(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(body): Json<BatchIsbnScanRequest>,
) -> Result<(StatusCode, Json<BatchIsbnScanResponse>), ApiError> {
    let mut tasks = Vec::with_capacity(body.book_ids.len());

    for book_id in &body.book_ids {
        let payload = serde_json::json!({
            "book_id": book_id.to_string(),
        });

        let task_id = state
            .task_queue()
            .enqueue(TaskType::ScanIsbn, payload)
            .await
            .map_err(|e| ApiError::Core(archivis_core::errors::ArchivisError::from(e)))?;

        tasks.push(IsbnScanResponse { task_id });
    }

    Ok((StatusCode::ACCEPTED, Json(BatchIsbnScanResponse { tasks })))
}
