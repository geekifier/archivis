pub mod handlers;
pub mod types;

use axum::routing::post;
use axum::Router;

use crate::state::AppState;

/// ISBN scan router mounted at `/api/isbn-scan`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/book/{id}", post(handlers::scan_book_isbn))
        .route("/batch", post(handlers::batch_scan_isbn))
}
