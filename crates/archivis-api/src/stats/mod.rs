pub mod handlers;
pub mod types;

use axum::{routing::get, Router};

use crate::state::AppState;

/// Stats router mounted at `/api/stats`.
pub fn router() -> Router<AppState> {
    Router::new().route("/", get(handlers::get_stats))
}
