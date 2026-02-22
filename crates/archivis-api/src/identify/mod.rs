pub mod handlers;
pub mod types;

use axum::routing::post;
use axum::Router;

use crate::state::AppState;

/// Identify router mounted at `/api/identify`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/batch", post(handlers::batch_identify))
        .route("/all", post(handlers::identify_all))
}
