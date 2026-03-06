pub(crate) mod handlers;
pub mod types;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::duplicates::handlers as duplicate_handlers;
use crate::resolve::handlers as resolution_handlers;
use crate::state::AppState;

/// Books router mounted at `/api/books`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::list_books))
        // Batch routes (no {id} path param)
        .route("/batch-update", post(handlers::batch_update_books))
        .route("/batch-tags", post(handlers::batch_set_tags))
        .route(
            "/refresh-metadata/batch",
            post(resolution_handlers::batch_refresh_metadata),
        )
        .route(
            "/refresh-metadata/all",
            post(resolution_handlers::refresh_all_metadata),
        )
        .route("/{id}", get(handlers::get_book))
        .route("/{id}", put(handlers::update_book))
        .route("/{id}", delete(handlers::delete_book))
        .route(
            "/{id}/refresh-metadata",
            post(resolution_handlers::refresh_metadata),
        )
        .route("/{id}/lock", post(handlers::lock_metadata))
        .route("/{id}/unlock", post(handlers::unlock_metadata))
        .route("/{id}/protect-fields", post(handlers::protect_fields))
        .route("/{id}/unprotect-fields", post(handlers::unprotect_fields))
        .route("/{id}/override-status", post(handlers::override_status))
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
        // Resolution review routes
        .route(
            "/{id}/candidates",
            get(resolution_handlers::list_candidates),
        )
        .route(
            "/{id}/candidates/{candidate_id}/apply",
            post(resolution_handlers::apply_candidate),
        )
        .route(
            "/{id}/candidates/{candidate_id}/reject",
            post(resolution_handlers::reject_candidate),
        )
        .route(
            "/{id}/candidates/{candidate_id}/undo",
            post(resolution_handlers::undo_candidate),
        )
}
