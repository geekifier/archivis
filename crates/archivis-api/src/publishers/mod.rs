pub(crate) mod handlers;
pub mod types;

use axum::routing::get;
use axum::Router;

use crate::state::AppState;

/// Publishers router mounted at `/api/publishers`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            get(handlers::list_publishers).post(handlers::create_publisher),
        )
        .route(
            "/{id}",
            get(handlers::get_publisher)
                .put(handlers::update_publisher)
                .delete(handlers::delete_publisher),
        )
        .route("/{id}/books", get(handlers::list_publisher_books))
}
