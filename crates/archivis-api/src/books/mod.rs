pub(crate) mod handlers;
pub mod types;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::duplicates::handlers as duplicate_handlers;
use crate::identify::handlers as identify_handlers;
use crate::state::AppState;

/// Books router mounted at `/api/books`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::list_books))
        // Batch routes (no {id} path param)
        .route("/batch-update", post(handlers::batch_update_books))
        .route("/batch-tags", post(handlers::batch_set_tags))
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
        .route(
            "/{id}/files/{file_id}/content",
            get(handlers::serve_file_content),
        )
        .route("/{id}/authors", post(handlers::set_book_authors))
        .route("/{id}/series", post(handlers::set_book_series))
        .route("/{id}/tags", post(handlers::set_book_tags))
        // Identifier management routes
        .route("/{id}/identifiers", post(handlers::add_identifier))
        .route(
            "/{id}/identifiers/{identifier_id}",
            put(handlers::update_identifier).delete(handlers::delete_identifier),
        )
        // Duplicate flagging route
        .route("/{id}/duplicates", post(duplicate_handlers::flag_duplicate))
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
