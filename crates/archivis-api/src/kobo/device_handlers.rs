use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use archivis_core::models::KoboDevice;
use archivis_db::KoboDeviceRepository;

use crate::auth::AuthUser;
use crate::errors::ApiError;
use crate::kobo::extractor::{generate_kobo_token, hash_kobo_token};
use crate::kobo::types::{
    DeviceResponse, KoboStatusResponse, PairDeviceRequest, PairDeviceResponse,
};
use crate::state::AppState;

const DEFAULT_DISPLAY_NAME: &str = "Kobo eReader";

/// `POST /api/kobo/devices` — pair a new Kobo device.
///
/// Returns the raw token exactly once. Subsequent reads only return device
/// metadata (never the token).
#[utoipa::path(
    post,
    path = "/api/kobo/devices",
    tag = "kobo",
    request_body = PairDeviceRequest,
    responses(
        (status = 201, description = "Device paired", body = PairDeviceResponse),
        (status = 401, description = "Not authenticated"),
        (status = 409, description = "public_base_url is not configured"),
    ),
    security(("bearer" = []))
)]
pub async fn pair_device(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(body): Json<PairDeviceRequest>,
) -> Result<(StatusCode, Json<PairDeviceResponse>), ApiError> {
    if !crate::kobo::sync_enabled(&state) {
        return Err(crate::kobo::sync_disabled_error());
    }

    let public_base_url = state.config().public_base_url.clone().ok_or_else(|| {
        ApiError::Conflict("public_base_url must be configured before pairing a Kobo device".into())
    })?;

    let token = generate_kobo_token();
    let token_hash = hash_kobo_token(&token);

    let display_name = body
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_DISPLAY_NAME)
        .to_string();

    let device = KoboDevice {
        id: Uuid::new_v4(),
        user_id: user.id,
        token_hash,
        display_name: display_name.clone(),
        created_at: Utc::now(),
        last_seen_at: None,
        revoked_at: None,
    };

    KoboDeviceRepository::create(state.db_pool(), &device).await?;

    let api_endpoint = public_base_url
        .join_path(&format!("/kobo/{token}"))
        .map_err(|e| ApiError::Internal(format!("failed to build api_endpoint: {e}")))?
        .to_string();

    Ok((
        StatusCode::CREATED,
        Json(PairDeviceResponse {
            id: device.id,
            display_name,
            token,
            api_endpoint,
        }),
    ))
}

/// `GET /api/kobo/devices` — list the calling user's devices (no token).
#[utoipa::path(
    get,
    path = "/api/kobo/devices",
    tag = "kobo",
    responses(
        (status = 200, description = "Devices", body = [DeviceResponse]),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn list_devices(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<DeviceResponse>>, ApiError> {
    let devices = KoboDeviceRepository::list_for_user(state.db_pool(), user.id).await?;
    Ok(Json(devices.into_iter().map(into_response).collect()))
}

/// `GET /api/kobo/status` — read global Kobo availability plus this user's
/// device count.
#[utoipa::path(
    get,
    path = "/api/kobo/status",
    tag = "kobo",
    responses(
        (status = 200, description = "Kobo status", body = KoboStatusResponse),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn status(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<KoboStatusResponse>, ApiError> {
    let devices = KoboDeviceRepository::list_for_user(state.db_pool(), user.id).await?;
    let active_device_count = devices.iter().filter(|d| d.revoked_at.is_none()).count();

    Ok(Json(KoboStatusResponse {
        enabled: crate::kobo::sync_enabled(&state),
        active_device_count,
        device_count: devices.len(),
    }))
}

/// `DELETE /api/kobo/devices/{device_id}` — revoke a device.
///
/// Idempotent from the user's perspective: returns 204 even if the device
/// was already revoked, as long as it belongs to the caller.
#[utoipa::path(
    delete,
    path = "/api/kobo/devices/{device_id}",
    tag = "kobo",
    params(("device_id" = Uuid, Path, description = "Device ID")),
    responses(
        (status = 204, description = "Revoked"),
        (status = 401, description = "Not authenticated"),
        (status = 404, description = "Device not found"),
    ),
    security(("bearer" = []))
)]
pub async fn revoke_device(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(device_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db_pool();

    // Confirm the device belongs to the calling user before revoking.
    let device = KoboDeviceRepository::get_by_id(pool, device_id).await?;
    if device.user_id != user.id {
        return Err(ApiError::NotFound("device not found".into()));
    }

    KoboDeviceRepository::revoke(pool, device_id, user.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn into_response(device: KoboDevice) -> DeviceResponse {
    DeviceResponse {
        id: device.id,
        display_name: device.display_name,
        created_at: device.created_at,
        last_seen_at: device.last_seen_at,
        revoked_at: device.revoked_at,
    }
}
