pub mod handlers;
pub mod types;

use axum::{routing::get, Router};

use crate::state::AppState;

/// Filesystem router mounted at `/api/filesystem`.
pub fn router() -> Router<AppState> {
    Router::new().route("/browse", get(handlers::browse_directory))
}
