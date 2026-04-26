use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;
use validator::Validate;

use crate::auth::types::UserResponse;
use crate::auth::{AuthUser, RequireAdmin};
use crate::errors::ApiError;
use crate::state::AppState;

use super::types::{
    AdminResetPasswordRequest, ChangePasswordRequest, CreateUserRequest, UpdateUserRequest,
};

/// GET /api/users -- list all users.
#[utoipa::path(
    get,
    path = "/api/users",
    tag = "users",
    responses(
        (status = 200, description = "List of users", body = Vec<UserResponse>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
    ),
    security(("bearer" = []))
)]
pub async fn list_users(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
) -> Result<Json<Vec<UserResponse>>, ApiError> {
    let users = state.auth_service().list_users().await?;
    Ok(Json(users.into_iter().map(Into::into).collect()))
}

/// POST /api/users -- create a new user.
#[utoipa::path(
    post,
    path = "/api/users",
    tag = "users",
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created", body = UserResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
        (status = 409, description = "Username already exists"),
    ),
    security(("bearer" = []))
)]
pub async fn create_user(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Json(body): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    body.validate()?;

    let user = state
        .auth_service()
        .create_user(
            &body.username,
            &body.password,
            body.email.as_deref(),
            body.role,
        )
        .await?;

    Ok((StatusCode::CREATED, Json(user.into())))
}

/// GET /api/users/{id} -- get user by ID.
#[utoipa::path(
    get,
    path = "/api/users/{id}",
    tag = "users",
    params(("id" = Uuid, Path, description = "User ID")),
    responses(
        (status = 200, description = "User detail", body = UserResponse),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "User not found"),
    ),
    security(("bearer" = []))
)]
pub async fn get_user(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<Uuid>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = state.auth_service().get_user(id).await?;
    Ok(Json(user.into()))
}

/// PUT /api/users/{id} -- update user (role, email, `is_active`).
#[utoipa::path(
    put,
    path = "/api/users/{id}",
    tag = "users",
    params(("id" = Uuid, Path, description = "User ID")),
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "Updated user", body = UserResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "User not found"),
        (status = 409, description = "Username already exists"),
    ),
    security(("bearer" = []))
)]
pub async fn update_user(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    body.validate()?;

    let user = state
        .auth_service()
        .update_user(
            id,
            body.username.as_deref(),
            body.email.as_ref().map(|e| e.as_deref()),
            body.role,
            body.is_active,
        )
        .await?;

    Ok(Json(user.into()))
}

/// DELETE /api/users/{id} -- deactivate user.
#[utoipa::path(
    delete,
    path = "/api/users/{id}",
    tag = "users",
    params(("id" = Uuid, Path, description = "User ID")),
    responses(
        (status = 204, description = "User deactivated"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required / cannot deactivate last admin"),
        (status = 404, description = "User not found"),
    ),
    security(("bearer" = []))
)]
pub async fn delete_user(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.auth_service().delete_user(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// PUT /api/users/{id}/password -- admin reset password.
#[utoipa::path(
    put,
    path = "/api/users/{id}/password",
    tag = "users",
    params(("id" = Uuid, Path, description = "User ID")),
    request_body = AdminResetPasswordRequest,
    responses(
        (status = 204, description = "Password reset"),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "User not found"),
    ),
    security(("bearer" = []))
)]
pub async fn admin_reset_password(
    State(state): State<AppState>,
    RequireAdmin(_admin): RequireAdmin,
    Path(id): Path<Uuid>,
    Json(body): Json<AdminResetPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    body.validate()?;

    state
        .auth_service()
        .admin_reset_password(id, &body.new_password)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// PUT /api/auth/password -- change own password (self-service).
#[utoipa::path(
    put,
    path = "/api/auth/password",
    tag = "auth",
    request_body = ChangePasswordRequest,
    responses(
        (status = 204, description = "Password changed"),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Not authenticated / wrong current password"),
    ),
    security(("bearer" = []))
)]
pub async fn change_password(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    body.validate()?;

    state
        .auth_service()
        .change_password(user.id, &body.current_password, &body.new_password)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ApiConfig;
    use axum::body::Body;
    use http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Stub settings reader that returns `None` for all keys.
    struct TestSettings;
    impl archivis_core::settings::SettingsReader for TestSettings {
        fn get_setting(&self, _key: &str) -> Option<serde_json::Value> {
            None
        }
    }

    async fn test_state(dir: &std::path::Path) -> AppState {
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
        let config_service = Arc::new(crate::settings::service::ConfigService::for_tests(
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
                public_base_url: None,
            },
            config_service,
            Arc::new(archivis_formats::transform::TransformerRegistry::empty()),
            None,
            None,
            [0u8; 32],
        )
    }

    /// Helper: register an admin, login, and return the bearer token.
    async fn admin_token(state: &AppState) -> String {
        state
            .auth_service()
            .register("admin", "adminpassword1", None)
            .await
            .unwrap();
        let (token, _) = state
            .auth_service()
            .login("admin", "adminpassword1")
            .await
            .unwrap();
        token
    }

    /// Helper: create a regular user and return a bearer token for them.
    async fn user_token(state: &AppState) -> String {
        state
            .auth_service()
            .register("regular", "userpassword1", None)
            .await
            .unwrap();
        let (token, _) = state
            .auth_service()
            .login("regular", "userpassword1")
            .await
            .unwrap();
        token
    }

    fn build_app(state: AppState) -> axum::Router {
        crate::build_router(state)
    }

    // ── List users ──────────────────────────────────────────────────

    #[tokio::test]
    async fn list_users_admin_gets_list() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;
        let token = admin_token(&state).await;
        let app = build_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/users")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(!json.as_array().unwrap().is_empty());
    }

    // ── Create user ─────────────────────────────────────────────────

    #[tokio::test]
    async fn create_user_valid_returns_201() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;
        let token = admin_token(&state).await;
        let app = build_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/users")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "username": "newuser",
                            "password": "strongpassword",
                            "role": "user"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["username"], "newuser");
        assert_eq!(json["role"], "user");
        assert!(json.get("password_hash").is_none());
    }

    #[tokio::test]
    async fn create_user_duplicate_returns_409() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;
        let token = admin_token(&state).await;
        let app = build_app(state);

        let body_json = serde_json::json!({
            "username": "admin",
            "password": "strongpassword",
            "role": "user"
        })
        .to_string();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/users")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body_json))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn create_user_weak_password_returns_400() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;
        let token = admin_token(&state).await;
        let app = build_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/users")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "username": "newuser",
                            "password": "short",
                            "role": "user"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ── Get user by ID ──────────────────────────────────────────────

    #[tokio::test]
    async fn get_user_found() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;
        let token = admin_token(&state).await;

        let created = state
            .auth_service()
            .create_user(
                "alice",
                "password123",
                None,
                archivis_core::models::UserRole::User,
            )
            .await
            .unwrap();

        let app = build_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/users/{}", created.id))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["username"], "alice");
    }

    #[tokio::test]
    async fn get_user_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;
        let token = admin_token(&state).await;
        let app = build_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/users/{}", Uuid::new_v4()))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ── Update user ─────────────────────────────────────────────────

    #[tokio::test]
    async fn update_user_change_role_and_email() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;
        let token = admin_token(&state).await;

        let created = state
            .auth_service()
            .create_user(
                "alice",
                "password123",
                None,
                archivis_core::models::UserRole::User,
            )
            .await
            .unwrap();

        let app = build_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/users/{}", created.id))
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "role": "admin",
                            "email": "alice@example.com"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["role"], "admin");
        assert_eq!(json["email"], "alice@example.com");
    }

    // ── Deactivate user via DELETE ──────────────────────────────────

    #[tokio::test]
    async fn delete_user_deactivates() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;
        let token = admin_token(&state).await;

        let created = state
            .auth_service()
            .create_user(
                "alice",
                "password123",
                None,
                archivis_core::models::UserRole::User,
            )
            .await
            .unwrap();

        let app = build_app(state.clone());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/users/{}", created.id))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Verify user is deactivated
        let user = state.auth_service().get_user(created.id).await.unwrap();
        assert!(!user.is_active);
    }

    #[tokio::test]
    async fn delete_user_cannot_deactivate_last_admin() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;
        let token = admin_token(&state).await;

        // Get the admin user's ID
        let users = state.auth_service().list_users().await.unwrap();
        let admin_id = users[0].id;

        let app = build_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/users/{admin_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ── Admin reset password ────────────────────────────────────────

    #[tokio::test]
    async fn admin_reset_password_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;
        let token = admin_token(&state).await;

        let created = state
            .auth_service()
            .create_user(
                "alice",
                "password123",
                None,
                archivis_core::models::UserRole::User,
            )
            .await
            .unwrap();

        let app = build_app(state.clone());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/users/{}/password", created.id))
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "new_password": "newstrongpassword" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Verify login with new password works
        let result = state
            .auth_service()
            .login("alice", "newstrongpassword")
            .await;
        assert!(result.is_ok());
    }

    // ── Self-service password change ────────────────────────────────

    #[tokio::test]
    async fn change_password_correct_old_password() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;

        state
            .auth_service()
            .register("alice", "oldpassword1", None)
            .await
            .unwrap();
        let (token, _) = state
            .auth_service()
            .login("alice", "oldpassword1")
            .await
            .unwrap();

        let app = build_app(state.clone());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/auth/password")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "current_password": "oldpassword1",
                            "new_password": "newpassword1"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Verify login with new password works
        let result = state.auth_service().login("alice", "newpassword1").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn change_password_wrong_old_password() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;

        state
            .auth_service()
            .register("alice", "oldpassword1", None)
            .await
            .unwrap();
        let (token, _) = state
            .auth_service()
            .login("alice", "oldpassword1")
            .await
            .unwrap();

        let app = build_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/auth/password")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "current_password": "wrongpassword",
                            "new_password": "newpassword1"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── Non-admin cannot access user management ─────────────────────

    #[tokio::test]
    async fn non_admin_cannot_list_users() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;
        // Create admin first (first user is admin)
        state
            .auth_service()
            .register("admin", "adminpassword1", None)
            .await
            .unwrap();
        let token = user_token(&state).await;

        let app = build_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/users")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn non_admin_cannot_create_user() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path()).await;
        state
            .auth_service()
            .register("admin", "adminpassword1", None)
            .await
            .unwrap();
        let token = user_token(&state).await;

        let app = build_app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/users")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "username": "another",
                            "password": "strongpassword",
                            "role": "user"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}
