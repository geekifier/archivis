pub mod handlers;
pub mod sse;
pub mod types;

use axum::{routing::get, Router};

use crate::state::AppState;

/// Task router mounted at `/api/tasks`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::list_tasks))
        .route("/active", get(sse::active_tasks_sse))
        .route("/{id}", get(handlers::get_task))
        .route("/{id}/progress", get(sse::task_progress_sse))
}
