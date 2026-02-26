pub(crate) mod handlers;
pub mod types;

use axum::routing::{get, put};
use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/progress/{book_id}",
            get(handlers::get_progress).delete(handlers::delete_progress),
        )
        .route(
            "/progress/{book_id}/{file_id}",
            put(handlers::update_progress),
        )
        .route("/continue-reading", get(handlers::continue_reading))
        .route(
            "/bookmarks/{book_id}/{file_id}",
            get(handlers::list_bookmarks).post(handlers::create_bookmark),
        )
        .route(
            "/bookmarks/{bookmark_id}",
            put(handlers::update_bookmark).delete(handlers::delete_bookmark),
        )
}
