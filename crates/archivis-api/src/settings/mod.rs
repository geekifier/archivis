pub mod handlers;
pub mod registry;
pub mod service;
pub mod types;

use axum::{routing::get, Router};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/",
        get(handlers::get_settings).put(handlers::update_settings),
    )
}
