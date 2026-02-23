use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Response from triggering an ISBN content scan for a book.
#[derive(Debug, Serialize, ToSchema)]
pub struct IsbnScanResponse {
    /// Background task ID for SSE progress tracking.
    pub task_id: Uuid,
}

/// Request body for batch ISBN content scanning.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchIsbnScanRequest {
    pub book_ids: Vec<Uuid>,
}

/// Response from batch ISBN content scanning.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchIsbnScanResponse {
    /// One task per scanned book.
    pub tasks: Vec<IsbnScanResponse>,
}
