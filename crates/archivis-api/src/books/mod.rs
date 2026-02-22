pub(crate) mod handlers;
pub mod types;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::identify::handlers as identify_handlers;
use crate::state::AppState;

/// Books router mounted at `/api/books`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::list_books))
        .route("/{id}", get(handlers::get_book))
        .route("/{id}", put(handlers::update_book))
        .route("/{id}", delete(handlers::delete_book))
        .route(
            "/{id}/cover",
            get(handlers::get_cover).post(handlers::upload_cover),
        )
        .route(
            "/{id}/files/{file_id}/download",
            get(handlers::download_file),
        )
        .route("/{id}/authors", post(handlers::set_book_authors))
        .route("/{id}/series", post(handlers::set_book_series))
        .route("/{id}/tags", post(handlers::set_book_tags))
        // Identification routes
        .route("/{id}/identify", post(identify_handlers::identify_book))
        .route("/{id}/candidates", get(identify_handlers::list_candidates))
        .route(
            "/{id}/candidates/{candidate_id}/apply",
            post(identify_handlers::apply_candidate),
        )
        .route(
            "/{id}/candidates/{candidate_id}/reject",
            post(identify_handlers::reject_candidate),
        )
        .route(
            "/{id}/candidates/{candidate_id}/undo",
            post(identify_handlers::undo_candidate),
        )
}
