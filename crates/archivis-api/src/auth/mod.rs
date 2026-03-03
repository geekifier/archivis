mod extractors;
pub(crate) mod handlers;
pub mod types;

pub use extractors::{AuthUser, RequireAdmin};

use axum::routing::{get, post, put};
use axum::Router;

use crate::state::AppState;

/// Auth router mounted at `/api/auth`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/status", get(handlers::auth_status))
        .route("/setup", post(handlers::auth_setup))
        .route("/login", post(handlers::auth_login))
        .route("/logout", post(handlers::auth_logout))
        .route("/me", get(handlers::auth_me))
        .route("/password", put(crate::users::handlers::change_password))
}
