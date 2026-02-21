use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::AppendHeaders;
use axum::Json;
use validator::Validate;

use crate::auth::extractors::AuthUser;
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{AuthStatusResponse, LoginRequest, LoginResponse, SetupRequest, UserResponse};

/// GET /api/auth/status -- check whether initial setup is required.
#[utoipa::path(
    get,
    path = "/api/auth/status",
    tag = "auth",
    responses(
        (status = 200, description = "Setup status", body = AuthStatusResponse),
    )
)]
pub async fn auth_status(
    State(state): State<AppState>,
) -> Result<Json<AuthStatusResponse>, ApiError> {
    let setup_required = state.auth_service().is_setup_required().await?;
    Ok(Json(AuthStatusResponse { setup_required }))
}

/// POST /api/auth/setup -- create the initial admin user.
#[utoipa::path(
    post,
    path = "/api/auth/setup",
    tag = "auth",
    request_body = SetupRequest,
    responses(
        (status = 201, description = "Admin user created", body = UserResponse),
        (status = 403, description = "Setup already completed"),
    )
)]
pub async fn auth_setup(
    State(state): State<AppState>,
    Json(body): Json<SetupRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    body.validate()?;

    let setup_required = state.auth_service().is_setup_required().await?;
    if !setup_required {
        return Err(ApiError::Forbidden);
    }

    let user = state
        .auth_service()
        .register(&body.username, &body.password, body.email.as_deref())
        .await?;

    Ok((StatusCode::CREATED, Json(user.into())))
}

/// POST /api/auth/login -- authenticate and receive a session token.
#[utoipa::path(
    post,
    path = "/api/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = LoginResponse),
        (status = 401, description = "Invalid credentials"),
    )
)]
pub async fn auth_login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<
    (
        AppendHeaders<[(axum::http::HeaderName, String); 1]>,
        Json<LoginResponse>,
    ),
    ApiError,
> {
    body.validate()?;

    let (raw_token, _session) = state
        .auth_service()
        .login(&body.username, &body.password)
        .await?;

    let cookie = format!("session={raw_token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000");

    let user = state.auth_service().validate_session(&raw_token).await?;

    Ok((
        AppendHeaders([(SET_COOKIE, cookie)]),
        Json(LoginResponse {
            token: raw_token,
            user: user.into(),
        }),
    ))
}

/// POST /api/auth/logout -- invalidate the current session.
#[utoipa::path(
    post,
    path = "/api/auth/logout",
    tag = "auth",
    responses(
        (status = 204, description = "Logged out"),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn auth_logout(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    headers: HeaderMap,
) -> Result<
    (
        StatusCode,
        AppendHeaders<[(axum::http::HeaderName, String); 1]>,
    ),
    ApiError,
> {
    // Extract token using the same logic as the extractor.
    let token = extract_token_from_headers(&headers).ok_or(ApiError::Unauthorized)?;

    state.auth_service().logout(&token).await?;

    let clear_cookie = "session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0".to_string();

    Ok((
        StatusCode::NO_CONTENT,
        AppendHeaders([(SET_COOKIE, clear_cookie)]),
    ))
}

/// GET /api/auth/me -- return the current authenticated user.
#[utoipa::path(
    get,
    path = "/api/auth/me",
    tag = "auth",
    responses(
        (status = 200, description = "Current user", body = UserResponse),
        (status = 401, description = "Not authenticated"),
    ),
    security(("bearer" = []))
)]
pub async fn auth_me(AuthUser(user): AuthUser) -> Json<UserResponse> {
    Json(user.into())
}

/// Extract a session token from request headers.
///
/// Checks `Authorization: Bearer <token>` first, then falls back to the
/// `session=<token>` cookie.
fn extract_token_from_headers(headers: &HeaderMap) -> Option<String> {
    // Try Authorization header first
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(ToString::to_string);

    if token.is_some() {
        return token;
    }

    // Fallback to session cookie
    headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies
                .split(';')
                .map(str::trim)
                .find(|c| c.starts_with("session="))
                .and_then(|c| c.strip_prefix("session="))
                .map(ToString::to_string)
        })
}
