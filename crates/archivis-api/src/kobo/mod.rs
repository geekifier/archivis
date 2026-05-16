pub(crate) mod device_handlers;
pub(crate) mod extractor;
pub(crate) mod protocol_handlers;
pub(crate) mod selection_handlers;
pub(crate) mod sync_protocol;
pub mod types;

#[cfg(test)]
mod tests;

use axum::routing::{any, delete, get, post};
use axum::Router;

use archivis_core::settings::SettingsReaderExt;

use crate::errors::ApiError;
use crate::state::AppState;

pub(super) const KOBO_ENABLED_KEY: &str = "kobo.enabled";
pub(super) const KOBO_SYNC_PAGE_SIZE_KEY: &str = "kobo.sync_page_size";
pub(super) const DEFAULT_KOBO_SYNC_PAGE_SIZE: usize = 25;

pub(super) fn sync_enabled(state: &AppState) -> bool {
    state
        .config_service()
        .get_bool(KOBO_ENABLED_KEY)
        .unwrap_or(true)
}

pub(super) fn sync_disabled_error() -> ApiError {
    ApiError::ServiceUnavailable("Kobo Sync is disabled".into())
}

pub(super) fn sync_page_size(state: &AppState) -> usize {
    state
        .config_service()
        .get_usize(KOBO_SYNC_PAGE_SIZE_KEY)
        .unwrap_or(DEFAULT_KOBO_SYNC_PAGE_SIZE)
        .clamp(1, 100)
}

/// User-facing device-management router mounted at `/api/kobo`.
pub fn user_router() -> Router<AppState> {
    Router::new()
        .route("/status", get(device_handlers::status))
        .route(
            "/devices",
            get(device_handlers::list_devices).post(device_handlers::pair_device),
        )
        .route(
            "/devices/{device_id}",
            delete(device_handlers::revoke_device),
        )
}

/// Kobo protocol router mounted at top-level `/kobo`.
///
/// The `{token}` path param is parsed by the [`extractor::kobo_token_layer`]
/// middleware and stored in request extensions, so handlers extract only
/// [`extractor::KoboDeviceAuth`].
pub fn protocol_router() -> Router<AppState> {
    let inner = Router::new()
        .route("/v1/initialization", get(protocol_handlers::initialization))
        .route("/v1/library/sync", get(protocol_handlers::library_sync))
        .route(
            "/v1/library/{book_id}/metadata",
            get(protocol_handlers::library_metadata),
        )
        .route("/v1/auth/device", post(protocol_handlers::auth_device))
        .route("/v1/auth/refresh", post(protocol_handlers::auth_refresh))
        .route(
            "/download/{book_id}/{book_file_id}",
            get(protocol_handlers::download),
        )
        .route(
            "/v1/products/images/{image_id}/{width}/{height}/{is_greyscale}/image.jpg",
            get(protocol_handlers::product_image),
        )
        .route(
            "/v1/products/images/{image_id}/{width}/{height}/{quality}/{is_greyscale}/image.jpg",
            get(protocol_handlers::product_image_quality),
        )
        // Store-side stubs: see `protocol_handlers::stub_*`.
        .route(
            "/v1/user/profile",
            get(protocol_handlers::stub_user_profile),
        )
        .route(
            "/v1/user/loyalty/benefits",
            get(protocol_handlers::stub_empty_object),
        )
        .route(
            "/v1/user/recommendations",
            get(protocol_handlers::stub_empty_array),
        )
        .route(
            "/v1/user/wishlist",
            get(protocol_handlers::stub_empty_array),
        )
        .route("/v1/user/ratings", get(protocol_handlers::stub_empty_array))
        .route("/v1/user/reviews", get(protocol_handlers::stub_empty_array))
        .route(
            "/v1/user/subscriptions",
            get(protocol_handlers::stub_empty_object),
        )
        .route("/v1/deals", get(protocol_handlers::stub_empty_array))
        .route(
            "/v1/products/dailydeal",
            get(protocol_handlers::stub_empty_object),
        )
        .route(
            "/v1/products/featured",
            get(protocol_handlers::stub_empty_array),
        )
        .route(
            "/v1/products/tasteprofile",
            get(protocol_handlers::stub_empty_object),
        )
        .route("/v1/library/tags", get(protocol_handlers::stub_empty_array))
        .route(
            "/v1/configuration",
            get(protocol_handlers::stub_configuration),
        )
        .route(
            "/v1/analytics/gettests",
            post(protocol_handlers::stub_analytics_gettests),
        )
        .route(
            "/v1/analytics/event",
            post(protocol_handlers::stub_no_content),
        )
        // Kobo firmware rewrites many `storeapi.kobo.com` resources through
        // `api_endpoint`. Keep unknown store probes from becoming sync-failing
        // 404s until Archivis has a real optional Kobo Store proxy.
        .route("/{*path}", any(protocol_handlers::stub_store_fallback));

    Router::new()
        .nest("/{token}", inner)
        .layer(axum::middleware::from_fn(extractor::kobo_token_layer))
}
