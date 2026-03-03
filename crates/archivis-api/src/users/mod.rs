pub(crate) mod handlers;
pub mod types;

use axum::routing::{get, put};
use axum::Router;

use crate::state::AppState;

/// Users router mounted at `/api/users`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::list_users).post(handlers::create_user))
        .route(
            "/{id}",
            get(handlers::get_user)
                .put(handlers::update_user)
                .delete(handlers::delete_user),
        )
        .route("/{id}/password", put(handlers::admin_reset_password))
}
