use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use archivis_core::errors::DbError;
use archivis_core::models::{User, UserRole};
use archivis_db::UserRepository;

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
#[derive(Debug)]
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
        // Try proxy auth first if configured
        if let Some(proxy_auth) = state.proxy_auth() {
            if let Some(user) = try_proxy_auth(parts, state, proxy_auth).await {
                return Ok(Self(user));
            }
        }

        // Fall through to session-based auth
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
            .map_err(|e| match &e {
                archivis_core::errors::AuthError::Internal(_) => {
                    tracing::error!(error = %e, "session validation failed");
                    AuthRejection(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal server error".into(),
                    )
                }
                _ => AuthRejection(StatusCode::UNAUTHORIZED, e.to_string()),
            })?;

        Ok(Self(user))
    }
}

/// Attempt proxy-based authentication.
///
/// Returns `Some(user)` if the request comes from a trusted proxy and
/// contains valid user headers. Returns `None` to fall through to session
/// auth in all other cases (untrusted IP, missing headers, etc.).
async fn try_proxy_auth(
    parts: &Parts,
    state: &AppState,
    proxy_auth: &archivis_auth::ProxyAuth,
) -> Option<User> {
    // Extract client IP from `ConnectInfo`
    let connect_info = parts.extensions.get::<ConnectInfo<SocketAddr>>()?;
    let client_ip = connect_info.0.ip();

    if !proxy_auth.is_trusted_proxy(&client_ip) {
        return None;
    }

    let info = proxy_auth.extract_user_info(&parts.headers)?;

    let pool = state.db_pool();

    // Look up existing user or auto-create
    let user = match UserRepository::get_by_username(pool, &info.username).await {
        Ok(mut user) => {
            // Update email if changed
            if user.email.as_deref() != info.email.as_deref() {
                user.email = info.email;
                let _ = UserRepository::update(pool, &user).await;
            }
            user
        }
        Err(DbError::NotFound { .. }) => {
            // Don't auto-create if setup hasn't completed yet (no users exist).
            // Otherwise the auto-created `User` role account makes
            // `is_setup_required()` return false, permanently blocking admin bootstrap.
            let user_count = UserRepository::count(pool).await.unwrap_or(0);
            if user_count == 0 {
                tracing::warn!(
                    username = %info.username,
                    "proxy auth: skipping auto-create because setup is not completed"
                );
                return None;
            }

            // Auto-create with a random unguessable password hash
            let placeholder_password = uuid::Uuid::new_v4().to_string();
            let password_hash = match archivis_auth::hash_password(&placeholder_password) {
                Ok(h) => h,
                Err(e) => {
                    tracing::error!("failed to hash placeholder password for proxy user: {e}");
                    return None;
                }
            };

            let mut user = User::new(info.username, password_hash, UserRole::User);
            user.email = info.email;

            if let Err(e) = UserRepository::create(pool, &user).await {
                tracing::error!("failed to auto-create proxy user: {e}");
                return None;
            }

            tracing::info!(username = %user.username, role = %user.role, "auto-created proxy auth user");
            user
        }
        Err(e) => {
            tracing::error!("failed to look up proxy user: {e}");
            return None;
        }
    };

    if !user.is_active {
        return None;
    }

    Some(user)
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

    // ── Proxy auth integration tests ───────────────────────────

    use crate::state::ApiConfig;
    use std::sync::Arc;

    /// Stub settings reader for tests.
    struct TestSettings;
    impl archivis_core::settings::SettingsReader for TestSettings {
        fn get_setting(&self, _key: &str) -> Option<serde_json::Value> {
            None
        }
    }

    /// Build a test `AppState` with optional proxy auth.
    async fn test_state_with_proxy(
        dir: &std::path::Path,
        proxy_auth: Option<Arc<archivis_auth::ProxyAuth>>,
    ) -> AppState {
        let db_path = dir.join("test.db");
        let storage_dir = dir.join("books");

        let db_pool = archivis_db::create_pool(&db_path).await.unwrap();
        archivis_db::run_migrations(&db_pool).await.unwrap();

        let storage = archivis_storage::local::LocalStorage::new(&storage_dir)
            .await
            .unwrap();
        let auth_adapter = archivis_auth::LocalAuthAdapter::new(db_pool.clone());
        let auth_service = archivis_auth::AuthService::new(db_pool.clone(), auth_adapter);
        let (task_queue, _rx) = archivis_tasks::queue::TaskQueue::new(db_pool.clone());

        let provider_registry = Arc::new(archivis_metadata::ProviderRegistry::new());
        let resolver = Arc::new(archivis_metadata::MetadataResolver::new(
            Arc::clone(&provider_registry),
            Arc::new(TestSettings),
        ));
        let resolve_service = Arc::new(archivis_tasks::resolve::ResolutionService::new(
            db_pool.clone(),
            resolver,
            storage.clone(),
            dir.to_path_buf(),
        ));
        let merge_service = Arc::new(archivis_tasks::merge::MergeService::new(
            db_pool.clone(),
            storage.clone(),
            dir.to_path_buf(),
        ));
        let config_service = Arc::new(crate::settings::service::ConfigService::new(
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            db_pool.clone(),
        ));

        AppState::new(
            db_pool,
            Arc::new(task_queue),
            auth_service,
            storage,
            provider_registry,
            resolve_service,
            merge_service,
            ApiConfig {
                data_dir: dir.to_path_buf(),
                frontend_dir: None,
            },
            config_service,
            None,
            proxy_auth,
            [0u8; 32],
        )
    }

    fn make_proxy_auth(trusted: &[&str]) -> Arc<archivis_auth::ProxyAuth> {
        Arc::new(
            archivis_auth::ProxyAuth::new(
                &trusted.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
                "X-Forwarded-User".to_string(),
                Some("X-Forwarded-Email".to_string()),
                Some("X-Forwarded-Groups".to_string()),
            )
            .unwrap(),
        )
    }

    /// Helper to build request parts with `ConnectInfo` extension set.
    fn parts_with_connect_info(req: Request<()>, addr: SocketAddr) -> Parts {
        let (mut parts, _body) = req.into_parts();
        parts.extensions.insert(ConnectInfo(addr));
        parts
    }

    /// Simulate completed setup by creating an admin user.
    async fn complete_setup(state: &AppState) {
        state
            .auth_service()
            .register("admin", "adminpassword1", None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn proxy_auth_blocked_before_setup() {
        let tmp = tempfile::tempdir().unwrap();
        let proxy = make_proxy_auth(&["127.0.0.1"]);
        let state = test_state_with_proxy(tmp.path(), Some(proxy)).await;

        // No users yet — proxy auto-create must be blocked to protect admin bootstrap
        let req = Request::builder()
            .header("X-Forwarded-User", "proxyuser")
            .body(())
            .unwrap();

        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut parts = parts_with_connect_info(req, addr);

        let result = AuthUser::from_request_parts(&mut parts, &state).await;
        assert!(
            result.is_err(),
            "proxy auto-create should be blocked before setup"
        );
    }

    #[tokio::test]
    async fn proxy_auth_auto_creates_user_from_trusted_ip() {
        let tmp = tempfile::tempdir().unwrap();
        let proxy = make_proxy_auth(&["127.0.0.1"]);
        let state = test_state_with_proxy(tmp.path(), Some(proxy)).await;
        complete_setup(&state).await;

        let req = Request::builder()
            .header("X-Forwarded-User", "proxyuser")
            .header("X-Forwarded-Email", "proxy@example.com")
            .body(())
            .unwrap();

        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut parts = parts_with_connect_info(req, addr);

        let result = AuthUser::from_request_parts(&mut parts, &state).await;
        assert!(result.is_ok());

        let user = result.unwrap().0;
        assert_eq!(user.username, "proxyuser");
        assert_eq!(user.email.as_deref(), Some("proxy@example.com"));
        assert_eq!(user.role, UserRole::User);
        assert!(user.is_active);
    }

    #[tokio::test]
    async fn proxy_auth_untrusted_ip_falls_through() {
        let tmp = tempfile::tempdir().unwrap();
        let proxy = make_proxy_auth(&["10.0.0.0/8"]);
        let state = test_state_with_proxy(tmp.path(), Some(proxy)).await;

        // Request from untrusted IP with proxy headers — should fall through
        // to session auth and fail (no token)
        let req = Request::builder()
            .header("X-Forwarded-User", "spoofed")
            .body(())
            .unwrap();

        let addr: SocketAddr = "192.168.1.1:12345".parse().unwrap();
        let mut parts = parts_with_connect_info(req, addr);

        let result = AuthUser::from_request_parts(&mut parts, &state).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn proxy_auth_missing_user_header_falls_through() {
        let tmp = tempfile::tempdir().unwrap();
        let proxy = make_proxy_auth(&["127.0.0.1"]);
        let state = test_state_with_proxy(tmp.path(), Some(proxy)).await;

        // Request from trusted IP but without the user header
        let req = Request::builder().body(()).unwrap();

        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut parts = parts_with_connect_info(req, addr);

        let result = AuthUser::from_request_parts(&mut parts, &state).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn proxy_auth_existing_user_reused() {
        let tmp = tempfile::tempdir().unwrap();
        let proxy = make_proxy_auth(&["127.0.0.1"]);
        let state = test_state_with_proxy(tmp.path(), Some(proxy)).await;
        complete_setup(&state).await;

        // First request: auto-creates user
        let req = Request::builder()
            .header("X-Forwarded-User", "alice")
            .body(())
            .unwrap();
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut parts = parts_with_connect_info(req, addr);
        let first = AuthUser::from_request_parts(&mut parts, &state)
            .await
            .unwrap()
            .0;

        // Second request: reuses existing user
        let req = Request::builder()
            .header("X-Forwarded-User", "alice")
            .body(())
            .unwrap();
        let mut parts = parts_with_connect_info(req, addr);
        let second = AuthUser::from_request_parts(&mut parts, &state)
            .await
            .unwrap()
            .0;

        assert_eq!(first.id, second.id);
    }

    #[tokio::test]
    async fn proxy_auth_user_cannot_login_with_placeholder_password() {
        let tmp = tempfile::tempdir().unwrap();
        let proxy = make_proxy_auth(&["127.0.0.1"]);
        let state = test_state_with_proxy(tmp.path(), Some(proxy)).await;
        complete_setup(&state).await;

        // Auto-create via proxy
        let req = Request::builder()
            .header("X-Forwarded-User", "proxyonly")
            .body(())
            .unwrap();
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut parts = parts_with_connect_info(req, addr);
        AuthUser::from_request_parts(&mut parts, &state)
            .await
            .unwrap();

        // Attempt local login — must fail since the password is a random UUID
        let result = state.auth_service().login("proxyonly", "anything").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn proxy_auth_disabled_ignores_headers() {
        let tmp = tempfile::tempdir().unwrap();
        // No proxy auth configured
        let state = test_state_with_proxy(tmp.path(), None).await;

        let req = Request::builder()
            .header("X-Forwarded-User", "alice")
            .body(())
            .unwrap();

        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut parts = parts_with_connect_info(req, addr);

        // Should fall through to session auth (no token → error)
        let result = AuthUser::from_request_parts(&mut parts, &state).await;
        assert!(result.is_err());
    }
}
