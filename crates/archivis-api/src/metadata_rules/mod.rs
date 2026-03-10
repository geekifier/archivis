pub mod handlers;
pub mod types;

use axum::routing::{get, put};
use axum::Router;

use crate::state::AppState;

/// Metadata rules router mounted at `/api/metadata-rules`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            get(handlers::list_metadata_rules).post(handlers::create_metadata_rule),
        )
        .route(
            "/{id}",
            put(handlers::update_metadata_rule).delete(handlers::delete_metadata_rule),
        )
}
