pub mod handlers;
pub mod types;

use axum::routing::{get, post};
use axum::Router;

use crate::state::AppState;

/// Duplicates router mounted at `/api/duplicates`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::list_duplicates))
        .route("/count", get(handlers::count_duplicates))
        .route("/{id}", get(handlers::get_duplicate))
        .route("/{id}/merge", post(handlers::merge_duplicate))
        .route("/{id}/dismiss", post(handlers::dismiss_duplicate))
}
