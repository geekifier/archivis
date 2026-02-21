pub(crate) mod handlers;
pub mod types;

use axum::routing::get;
use axum::Router;

use crate::state::AppState;

/// Authors router mounted at `/api/authors`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            get(handlers::list_authors).post(handlers::create_author),
        )
        .route(
            "/{id}",
            get(handlers::get_author)
                .put(handlers::update_author)
                .delete(handlers::delete_author),
        )
        .route("/{id}/books", get(handlers::list_author_books))
}
