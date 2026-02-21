pub(crate) mod handlers;
pub mod types;

use axum::routing::get;
use axum::Router;

use crate::state::AppState;

/// Series router mounted at `/api/series`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            get(handlers::list_series).post(handlers::create_series),
        )
        .route(
            "/{id}",
            get(handlers::get_series)
                .put(handlers::update_series)
                .delete(handlers::delete_series),
        )
        .route("/{id}/books", get(handlers::list_series_books))
}
