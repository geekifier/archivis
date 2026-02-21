use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use archivis_core::models::{User, UserRole};

use crate::state::AppState;

/// Extractor that validates the session and provides the authenticated user.
///
/// Extracts the session token from:
/// 1. `Authorization: Bearer <token>` header
/// 2. `session=<token>` cookie
pub struct AuthUser(pub User);

impl AuthUser {
    pub fn user(&self) -> &User {
        &self.0
    }

    pub fn user_id(&self) -> uuid::Uuid {
        self.0.id
    }
}

/// Rejection type for auth extraction failures.
pub struct AuthRejection(pub StatusCode, pub String);

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": {
                "status": self.0.as_u16(),
                "message": self.1,
            }
        });
        (self.0, axum::Json(body)).into_response()
    }
}

/// Extract a session token from request headers.
///
/// Checks `Authorization: Bearer <token>` first, then falls back to
/// the `session=<token>` cookie.
fn extract_token(parts: &Parts) -> Option<String> {
    // Try Authorization header first
    let token = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(ToString::to_string);

    if token.is_some() {
        return token;
    }

    // Fallback to session cookie
    parts
        .headers
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

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_token(parts).ok_or_else(|| {
            AuthRejection(
                StatusCode::UNAUTHORIZED,
                "missing authentication token".into(),
            )
        })?;

        let user = state
            .auth_service()
            .validate_session(&token)
            .await
            .map_err(|e| AuthRejection(StatusCode::UNAUTHORIZED, e.to_string()))?;

        Ok(Self(user))
    }
}

/// Extractor that requires the authenticated user to have the Admin role.
///
/// Extracts `AuthUser` first, then checks the role.
pub struct RequireAdmin(pub User);

impl FromRequestParts<AppState> for RequireAdmin {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let AuthUser(user) = AuthUser::from_request_parts(parts, state).await?;

        if user.role != UserRole::Admin {
            return Err(AuthRejection(
                StatusCode::FORBIDDEN,
                "admin access required".into(),
            ));
        }

        Ok(Self(user))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    fn parts_from_request(req: Request<()>) -> Parts {
        req.into_parts().0
    }

    #[test]
    fn extract_token_from_bearer_header() {
        let req = Request::builder()
            .header("authorization", "Bearer my_test_token_123")
            .body(())
            .unwrap();
        let parts = parts_from_request(req);
        assert_eq!(extract_token(&parts), Some("my_test_token_123".to_string()));
    }

    #[test]
    fn extract_token_from_session_cookie() {
        let req = Request::builder()
            .header("cookie", "session=cookie_token_456")
            .body(())
            .unwrap();
        let parts = parts_from_request(req);
        assert_eq!(extract_token(&parts), Some("cookie_token_456".to_string()));
    }

    #[test]
    fn extract_token_bearer_takes_precedence_over_cookie() {
        let req = Request::builder()
            .header("authorization", "Bearer bearer_token")
            .header("cookie", "session=cookie_token")
            .body(())
            .unwrap();
        let parts = parts_from_request(req);
        assert_eq!(extract_token(&parts), Some("bearer_token".to_string()));
    }

    #[test]
    fn extract_token_from_multiple_cookies() {
        let req = Request::builder()
            .header("cookie", "theme=dark; session=my_session_tok; lang=en")
            .body(())
            .unwrap();
        let parts = parts_from_request(req);
        assert_eq!(extract_token(&parts), Some("my_session_tok".to_string()));
    }

    #[test]
    fn extract_token_returns_none_without_credentials() {
        let req = Request::builder().body(()).unwrap();
        let parts = parts_from_request(req);
        assert_eq!(extract_token(&parts), None);
    }

    #[test]
    fn extract_token_ignores_non_bearer_auth() {
        let req = Request::builder()
            .header("authorization", "Basic dXNlcjpwYXNz")
            .body(())
            .unwrap();
        let parts = parts_from_request(req);
        assert_eq!(extract_token(&parts), None);
    }

    #[test]
    fn extract_token_ignores_unrelated_cookies() {
        let req = Request::builder()
            .header("cookie", "theme=dark; lang=en")
            .body(())
            .unwrap();
        let parts = parts_from_request(req);
        assert_eq!(extract_token(&parts), None);
    }

    #[test]
    fn auth_rejection_produces_json_response() {
        let rejection = AuthRejection(StatusCode::UNAUTHORIZED, "bad token".into());
        let response = rejection.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn auth_rejection_forbidden() {
        let rejection = AuthRejection(StatusCode::FORBIDDEN, "admin access required".into());
        let response = rejection.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
