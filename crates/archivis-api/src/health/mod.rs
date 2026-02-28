pub mod handlers;
pub mod types;

use axum::{routing::get, Router};

use crate::state::AppState;

/// Health-check router mounted at `/health`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/live", get(handlers::liveness))
        .route("/ready", get(handlers::readiness))
}
