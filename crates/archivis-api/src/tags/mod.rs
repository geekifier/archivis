pub(crate) mod handlers;
pub mod types;

use axum::routing::get;
use axum::Router;

use crate::state::AppState;

/// Tags router mounted at `/api/tags`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::list_tags).post(handlers::create_tag))
        .route("/categories", get(handlers::list_categories))
        .route(
            "/{id}",
            get(handlers::get_tag)
                .put(handlers::update_tag)
                .delete(handlers::delete_tag),
        )
        .route("/{id}/books", get(handlers::list_tag_books))
}
