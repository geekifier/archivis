pub mod handlers;
pub mod types;

use axum::extract::DefaultBodyLimit;
use axum::{routing::post, Router};

use crate::state::AppState;

/// Import router mounted at `/api/import`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/upload", post(handlers::upload_files))
        .route("/scan", post(handlers::scan_directory))
        .route("/scan/start", post(handlers::start_import))
        // Allow up to 512 MiB for ebook uploads.
        .layer(DefaultBodyLimit::max(512 * 1024 * 1024))
}
