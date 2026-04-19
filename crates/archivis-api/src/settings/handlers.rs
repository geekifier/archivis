use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::auth::RequireAdmin;
use crate::errors::ApiError;
use crate::state::AppState;

use super::service::SettingError;
use super::types::{
    SettingsResponse, UpdateSettingsErrorResponse, UpdateSettingsRequest, UpdateSettingsResponse,
};

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

/// Structured error response for `PUT /api/settings`.
///
/// `ConfigService::update` returns `Vec<SettingError>`. The handler wraps it
/// in `UpdateSettingsErrorResponse` and serializes with HTTP 400.
pub struct UpdateSettingsFailed(pub Vec<SettingError>);

impl IntoResponse for UpdateSettingsFailed {
    fn into_response(self) -> Response {
        let body = UpdateSettingsErrorResponse { errors: self.0 };
        (StatusCode::BAD_REQUEST, Json(body)).into_response()
    }
}

#[utoipa::path(
    put,
    path = "/api/settings",
    tag = "settings",
    security(("bearer" = [])),
    request_body = UpdateSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = UpdateSettingsResponse),
        (status = 400, description = "Validation failed", body = UpdateSettingsErrorResponse),
        (status = 403, description = "Admin access required"),
    )
)]
pub async fn update_settings(
    _admin: RequireAdmin,
    State(state): State<AppState>,
    Json(body): Json<UpdateSettingsRequest>,
) -> Result<Json<UpdateSettingsResponse>, UpdateSettingsFailed> {
    match state.config_service().update(&body.settings).await {
        Ok(result) => Ok(Json(UpdateSettingsResponse {
            updated: result.updated,
            requires_restart: result.requires_restart,
        })),
        Err(errors) => Err(UpdateSettingsFailed(errors)),
    }
}
