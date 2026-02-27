pub mod handlers;
pub mod types;

use axum::routing::{get, post};
use axum::Router;

use crate::state::AppState;

/// Watcher router mounted at `/api/watched-directories`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::list_watched).post(handlers::add_watched))
        .route(
            "/{id}",
            get(handlers::get_watched)
                .put(handlers::update_watched)
                .delete(handlers::remove_watched),
        )
        .route("/{id}/scan", post(handlers::trigger_scan))
        .route("/detect", post(handlers::detect_fs))
}
