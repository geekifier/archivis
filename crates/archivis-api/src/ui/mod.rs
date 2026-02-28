pub mod handlers;
pub mod types;

use axum::{routing::get, Router};

use crate::state::AppState;

/// UI helper routes mounted at `/api/ui`.
pub fn router() -> Router<AppState> {
    Router::new().route("/sidebar-counts", get(handlers::sidebar_counts))
}
