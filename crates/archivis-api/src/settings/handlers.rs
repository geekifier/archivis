use axum::extract::State;
use axum::Json;

use crate::auth::RequireAdmin;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{SettingsResponse, UpdateSettingsRequest, UpdateSettingsResponse};

#[utoipa::path(
    get,
    path = "/api/settings",
    tag = "settings",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "All instance settings", body = SettingsResponse),
        (status = 403, description = "Admin access required"),
    )
)]
pub async fn get_settings(
    _admin: RequireAdmin,
    State(state): State<AppState>,
) -> Result<Json<SettingsResponse>, ApiError> {
    let entries = state.config_service().get_all_entries();
    Ok(Json(SettingsResponse { settings: entries }))
}

#[utoipa::path(
    put,
    path = "/api/settings",
    tag = "settings",
    security(("bearer" = [])),
    request_body = UpdateSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = UpdateSettingsResponse),
        (status = 400, description = "Validation failed"),
        (status = 403, description = "Admin access required"),
    )
)]
pub async fn update_settings(
    _admin: RequireAdmin,
    State(state): State<AppState>,
    Json(body): Json<UpdateSettingsRequest>,
) -> Result<Json<UpdateSettingsResponse>, ApiError> {
    let result = state
        .config_service()
        .update(&body.settings)
        .await
        .map_err(ApiError::Validation)?;

    Ok(Json(UpdateSettingsResponse {
        updated: result.updated,
        requires_restart: result.requires_restart,
    }))
}
