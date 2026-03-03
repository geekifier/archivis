use chrono::Utc;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use uuid::Uuid;

use archivis_core::errors::{AuthError, DbError};
use archivis_core::models::{Session, User, UserRole};
use archivis_db::{SessionRepository, UserRepository};

use crate::adapter::AuthAdapter;
use crate::local;

/// High-level authentication service that coordinates between an [`AuthAdapter`]
/// and the session store.
pub struct AuthService<A: AuthAdapter> {
    pool: SqlitePool,
    adapter: A,
}

impl<A: AuthAdapter> AuthService<A> {
    pub fn new(pool: SqlitePool, adapter: A) -> Self {
        Self { pool, adapter }
    }

    /// Register a new user. First user automatically gets Admin role.
    pub async fn register(
        &self,
        username: &str,
        password: &str,
        email: Option<&str>,
    ) -> Result<User, AuthError> {
        self.adapter.register(username, password, email).await
    }

    /// Login with credentials. Returns `(raw_token, session)`.
    ///
    /// The raw token must be sent to the client; only its SHA-256 hash is stored.
    pub async fn login(
        &self,
        username: &str,
        password: &str,
    ) -> Result<(String, Session), AuthError> {
        let user = self.adapter.authenticate(username, password).await?;

        let raw_token = Self::generate_token();
        let token_hash = Self::hash_token(&raw_token);

        let expires_at = Utc::now() + chrono::Duration::days(30);
        let session = Session::new(user.id, token_hash, expires_at);

        SessionRepository::create(&self.pool, &session)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        Ok((raw_token, session))
    }

    /// Validate a session token and return the associated user.
    pub async fn validate_session(&self, token: &str) -> Result<User, AuthError> {
        let token_hash = Self::hash_token(token);

        let session = SessionRepository::get_by_token_hash(&self.pool, &token_hash)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthError::InvalidCredentials,
                other => AuthError::Internal(other.to_string()),
            })?;

        if session.expires_at < Utc::now() {
            let _ = SessionRepository::delete(&self.pool, session.id).await;
            return Err(AuthError::SessionExpired);
        }

        let user = UserRepository::get_by_id(&self.pool, session.user_id)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        if !user.is_active {
            return Err(AuthError::Unauthorized);
        }

        Ok(user)
    }

    /// Logout by invalidating the session token.
    pub async fn logout(&self, token: &str) -> Result<(), AuthError> {
        let token_hash = Self::hash_token(token);

        let session = match SessionRepository::get_by_token_hash(&self.pool, &token_hash).await {
            Ok(s) => s,
            Err(DbError::NotFound { .. }) => return Ok(()), // Already logged out, idempotent
            Err(e) => return Err(AuthError::Internal(e.to_string())),
        };

        SessionRepository::delete(&self.pool, session.id)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        Ok(())
    }

    /// Check if the application needs initial setup (no users exist).
    pub async fn is_setup_required(&self) -> Result<bool, AuthError> {
        let count = UserRepository::count(&self.pool)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        Ok(count == 0)
    }

    /// Create a new user with an explicit role (admin-invoked).
    pub async fn create_user(
        &self,
        username: &str,
        password: &str,
        email: Option<&str>,
        role: UserRole,
    ) -> Result<User, AuthError> {
        local::validate_password(password)?;

        // Check for duplicate username
        match UserRepository::get_by_username(&self.pool, username).await {
            Ok(_) => return Err(AuthError::UserExists(username.to_string())),
            Err(DbError::NotFound { .. }) => {}
            Err(e) => return Err(AuthError::Internal(e.to_string())),
        }

        // Hash password in blocking task
        let pwd = password.to_string();
        let password_hash = tokio::task::spawn_blocking(move || local::hash_password(&pwd))
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))??;

        let mut user = User::new(username.to_string(), password_hash, role);
        user.email = email.map(ToString::to_string);

        UserRepository::create(&self.pool, &user)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        Ok(user)
    }

    /// Return all users.
    pub async fn list_users(&self) -> Result<Vec<User>, AuthError> {
        UserRepository::list(&self.pool)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))
    }

    /// Get a single user by ID.
    pub async fn get_user(&self, id: Uuid) -> Result<User, AuthError> {
        UserRepository::get_by_id(&self.pool, id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthError::UserNotFound(id.to_string()),
                other => AuthError::Internal(other.to_string()),
            })
    }

    /// Update user fields. Invalidates sessions when the user is deactivated.
    pub async fn update_user(
        &self,
        id: Uuid,
        username: Option<&str>,
        email: Option<Option<&str>>,
        role: Option<UserRole>,
        is_active: Option<bool>,
    ) -> Result<User, AuthError> {
        let mut user = UserRepository::get_by_id(&self.pool, id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthError::UserNotFound(id.to_string()),
                other => AuthError::Internal(other.to_string()),
            })?;

        let was_active_admin = user.role == UserRole::Admin && user.is_active;

        if let Some(new_username) = username {
            if new_username != user.username {
                // Check uniqueness
                match UserRepository::get_by_username(&self.pool, new_username).await {
                    Ok(_) => return Err(AuthError::UserExists(new_username.to_string())),
                    Err(DbError::NotFound { .. }) => {}
                    Err(e) => return Err(AuthError::Internal(e.to_string())),
                }
                user.username = new_username.to_string();
            }
        }

        if let Some(new_email) = email {
            user.email = new_email.map(ToString::to_string);
        }

        if let Some(new_role) = role {
            user.role = new_role;
        }

        if let Some(active) = is_active {
            user.is_active = active;
        }

        // Prevent removing the last active admin via role demotion or deactivation.
        // The original user was an active admin if `was_active_admin` is true;
        // after applying changes, check whether they are still an active admin.
        if was_active_admin && !(user.role == UserRole::Admin && user.is_active) {
            let admin_count = UserRepository::count_by_role(&self.pool, UserRole::Admin)
                .await
                .map_err(|e| AuthError::Internal(e.to_string()))?;
            if admin_count <= 1 {
                return Err(AuthError::Forbidden);
            }
        }

        UserRepository::update(&self.pool, &user)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        // Invalidate sessions when user is deactivated
        if is_active == Some(false) {
            SessionRepository::delete_by_user(&self.pool, id)
                .await
                .map_err(|e| AuthError::Internal(e.to_string()))?;
        }

        Ok(user)
    }

    /// Self-service password change. Verifies the current password first.
    pub async fn change_password(
        &self,
        user_id: Uuid,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), AuthError> {
        let user = UserRepository::get_by_id(&self.pool, user_id)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        // Verify current password in blocking task
        let hash = user.password_hash.clone();
        let pwd = current_password.to_string();
        let valid = tokio::task::spawn_blocking(move || local::verify_password(&pwd, &hash))
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))??;

        if !valid {
            return Err(AuthError::InvalidCredentials);
        }

        local::validate_password(new_password)?;

        // Hash new password in blocking task
        let new_pwd = new_password.to_string();
        let new_hash = tokio::task::spawn_blocking(move || local::hash_password(&new_pwd))
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))??;

        UserRepository::update_password(&self.pool, user_id, &new_hash)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        // Invalidate all sessions
        SessionRepository::delete_by_user(&self.pool, user_id)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        Ok(())
    }

    /// Admin password reset. No current password needed.
    pub async fn admin_reset_password(
        &self,
        user_id: Uuid,
        new_password: &str,
    ) -> Result<(), AuthError> {
        // Verify user exists
        UserRepository::get_by_id(&self.pool, user_id)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        local::validate_password(new_password)?;

        let new_pwd = new_password.to_string();
        let new_hash = tokio::task::spawn_blocking(move || local::hash_password(&new_pwd))
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))??;

        UserRepository::update_password(&self.pool, user_id, &new_hash)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        // Invalidate all sessions
        SessionRepository::delete_by_user(&self.pool, user_id)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        Ok(())
    }

    /// Soft-delete a user (set `is_active=false`) and invalidate all sessions.
    ///
    /// Returns an error if this is the last active admin.
    pub async fn delete_user(&self, id: Uuid) -> Result<(), AuthError> {
        let user = UserRepository::get_by_id(&self.pool, id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthError::UserNotFound(id.to_string()),
                other => AuthError::Internal(other.to_string()),
            })?;

        // Prevent deactivating the last admin
        if user.role == UserRole::Admin && user.is_active {
            let admin_count = UserRepository::count_by_role(&self.pool, UserRole::Admin)
                .await
                .map_err(|e| AuthError::Internal(e.to_string()))?;
            if admin_count <= 1 {
                return Err(AuthError::Forbidden);
            }
        }

        let mut user = user;
        user.is_active = false;

        UserRepository::update(&self.pool, &user)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        SessionRepository::delete_by_user(&self.pool, id)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        Ok(())
    }

    /// Generate a cryptographically secure random token (32 bytes, hex-encoded).
    fn generate_token() -> String {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        bytes_to_hex(&bytes)
    }

    /// Hash a raw token with SHA-256 for storage.
    fn hash_token(token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        let result = hasher.finalize();
        bytes_to_hex(&result)
    }
}

/// Encode a byte slice as a lowercase hex string.
fn bytes_to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(s, "{b:02x}").expect("hex encoding cannot fail");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use archivis_db::{create_pool, run_migrations, DbPool};
    use tempfile::TempDir;

    /// Create a fresh test database and return the pool + `TempDir` guard.
    async fn test_pool() -> (DbPool, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let pool = create_pool(&db_path).await.unwrap();
        run_migrations(&pool).await.unwrap();
        (pool, dir)
    }

    /// Build an `AuthService<LocalAuthAdapter>` backed by the given pool.
    fn test_service(pool: &SqlitePool) -> AuthService<crate::LocalAuthAdapter> {
        let adapter = crate::LocalAuthAdapter::new(pool.clone());
        AuthService::new(pool.clone(), adapter)
    }

    #[test]
    fn generate_token_is_unique() {
        let t1 = AuthService::<crate::LocalAuthAdapter>::generate_token();
        let t2 = AuthService::<crate::LocalAuthAdapter>::generate_token();
        assert_ne!(t1, t2);
        assert_eq!(t1.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn hash_token_is_deterministic() {
        let hash1 = AuthService::<crate::LocalAuthAdapter>::hash_token("test_token");
        let hash2 = AuthService::<crate::LocalAuthAdapter>::hash_token("test_token");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn token_hash_is_not_raw_token() {
        let raw = "my_raw_token_value";
        let hash = AuthService::<crate::LocalAuthAdapter>::hash_token(raw);
        assert_ne!(hash, raw);
    }

    #[test]
    fn hash_token_output_is_hex() {
        let hash = AuthService::<crate::LocalAuthAdapter>::hash_token("anything");
        assert_eq!(hash.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn bytes_to_hex_encodes_correctly() {
        assert_eq!(bytes_to_hex(&[0x00, 0xff, 0xab, 0x12]), "00ffab12");
        assert_eq!(bytes_to_hex(&[]), "");
    }

    // ── User management tests ─────────────────────────────────────

    #[tokio::test]
    async fn create_user_with_explicit_role() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let user = svc
            .create_user("admin1", "strongpassword", Some("a@b.com"), UserRole::Admin)
            .await
            .unwrap();

        assert_eq!(user.username, "admin1");
        assert_eq!(user.role, UserRole::Admin);
        assert_eq!(user.email.as_deref(), Some("a@b.com"));
        assert!(user.is_active);
    }

    #[tokio::test]
    async fn create_user_duplicate_username() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        svc.create_user("alice", "strongpassword", None, UserRole::User)
            .await
            .unwrap();

        let err = svc
            .create_user("alice", "anotherpassword", None, UserRole::User)
            .await
            .unwrap_err();

        assert!(matches!(err, AuthError::UserExists(name) if name == "alice"));
    }

    #[tokio::test]
    async fn create_user_weak_password() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let err = svc
            .create_user("bob", "short", None, UserRole::User)
            .await
            .unwrap_err();

        assert!(matches!(err, AuthError::WeakPassword(_)));
    }

    #[tokio::test]
    async fn list_users_returns_all() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        svc.create_user("alice", "password123", None, UserRole::Admin)
            .await
            .unwrap();
        svc.create_user("bob", "password456", None, UserRole::User)
            .await
            .unwrap();

        let users = svc.list_users().await.unwrap();
        assert_eq!(users.len(), 2);
    }

    #[tokio::test]
    async fn get_user_by_id() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let created = svc
            .create_user("alice", "password123", None, UserRole::User)
            .await
            .unwrap();

        let fetched = svc.get_user(created.id).await.unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.username, "alice");
    }

    #[tokio::test]
    async fn get_user_not_found() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let err = svc.get_user(Uuid::new_v4()).await.unwrap_err();
        assert!(matches!(err, AuthError::UserNotFound(_)));
    }

    #[tokio::test]
    async fn update_user_role_and_email() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let user = svc
            .create_user("alice", "password123", None, UserRole::User)
            .await
            .unwrap();

        let updated = svc
            .update_user(
                user.id,
                None,
                Some(Some("new@email.com")),
                Some(UserRole::Admin),
                None,
            )
            .await
            .unwrap();

        assert_eq!(updated.role, UserRole::Admin);
        assert_eq!(updated.email.as_deref(), Some("new@email.com"));
    }

    #[tokio::test]
    async fn update_user_deactivate_invalidates_sessions() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        // Create a user and a manual session for them
        let target = svc
            .create_user("alice", "password123", None, UserRole::User)
            .await
            .unwrap();

        let expires_at = Utc::now() + chrono::Duration::days(30);
        let session = Session::new(target.id, "fakehash".to_string(), expires_at);
        SessionRepository::create(&pool, &session).await.unwrap();

        // Deactivate the user
        let updated = svc
            .update_user(target.id, None, None, None, Some(false))
            .await
            .unwrap();
        assert!(!updated.is_active);

        // Session should be gone
        let result = SessionRepository::get_by_token_hash(&pool, "fakehash").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn update_user_username_uniqueness() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        svc.create_user("alice", "password123", None, UserRole::User)
            .await
            .unwrap();
        let bob = svc
            .create_user("bob", "password123", None, UserRole::User)
            .await
            .unwrap();

        let err = svc
            .update_user(bob.id, Some("alice"), None, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, AuthError::UserExists(name) if name == "alice"));
    }

    #[tokio::test]
    async fn update_user_cannot_demote_last_admin() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let admin = svc
            .create_user("admin1", "password123", None, UserRole::Admin)
            .await
            .unwrap();

        let err = svc
            .update_user(admin.id, None, None, Some(UserRole::User), None)
            .await
            .unwrap_err();

        assert!(matches!(err, AuthError::Forbidden));
    }

    #[tokio::test]
    async fn update_user_cannot_deactivate_last_admin() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let admin = svc
            .create_user("admin1", "password123", None, UserRole::Admin)
            .await
            .unwrap();

        let err = svc
            .update_user(admin.id, None, None, None, Some(false))
            .await
            .unwrap_err();

        assert!(matches!(err, AuthError::Forbidden));
    }

    #[tokio::test]
    async fn update_user_allows_demoting_non_last_admin() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let admin1 = svc
            .create_user("admin1", "password123", None, UserRole::Admin)
            .await
            .unwrap();
        svc.create_user("admin2", "password123", None, UserRole::Admin)
            .await
            .unwrap();

        let updated = svc
            .update_user(admin1.id, None, None, Some(UserRole::User), None)
            .await
            .unwrap();

        assert_eq!(updated.role, UserRole::User);
    }

    #[tokio::test]
    async fn delete_user_cannot_deactivate_last_admin() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let admin = svc
            .create_user("admin1", "password123", None, UserRole::Admin)
            .await
            .unwrap();

        let err = svc.delete_user(admin.id).await.unwrap_err();
        assert!(matches!(err, AuthError::Forbidden));
    }

    #[tokio::test]
    async fn delete_user_allows_deactivating_non_last_admin() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let admin1 = svc
            .create_user("admin1", "password123", None, UserRole::Admin)
            .await
            .unwrap();
        svc.create_user("admin2", "password123", None, UserRole::Admin)
            .await
            .unwrap();

        svc.delete_user(admin1.id).await.unwrap();

        let deactivated = svc.get_user(admin1.id).await.unwrap();
        assert!(!deactivated.is_active);
    }

    #[tokio::test]
    async fn change_password_correct_old_password() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        // Use `register` so the password is hashed through the adapter
        let user = svc.register("alice", "oldpassword1", None).await.unwrap();

        svc.change_password(user.id, "oldpassword1", "newpassword1")
            .await
            .unwrap();

        // Should be able to login with the new password
        let result = svc.login("alice", "newpassword1").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn change_password_wrong_old_password() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let user = svc.register("alice", "oldpassword1", None).await.unwrap();

        let err = svc
            .change_password(user.id, "wrongpassword", "newpassword1")
            .await
            .unwrap_err();

        assert!(matches!(err, AuthError::InvalidCredentials));
    }

    #[tokio::test]
    async fn change_password_weak_new_password() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let user = svc.register("alice", "oldpassword1", None).await.unwrap();

        let err = svc
            .change_password(user.id, "oldpassword1", "short")
            .await
            .unwrap_err();

        assert!(matches!(err, AuthError::WeakPassword(_)));
    }

    #[tokio::test]
    async fn change_password_invalidates_sessions() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let user = svc.register("alice", "oldpassword1", None).await.unwrap();
        let (token, _session) = svc.login("alice", "oldpassword1").await.unwrap();

        // Password change should invalidate the session
        svc.change_password(user.id, "oldpassword1", "newpassword1")
            .await
            .unwrap();

        let err = svc.validate_session(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidCredentials));
    }

    #[tokio::test]
    async fn admin_reset_password() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let user = svc.register("alice", "oldpassword1", None).await.unwrap();

        svc.admin_reset_password(user.id, "adminsetpassword1")
            .await
            .unwrap();

        // Should be able to login with the new password
        let result = svc.login("alice", "adminsetpassword1").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn admin_reset_password_invalidates_sessions() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let user = svc.register("alice", "oldpassword1", None).await.unwrap();
        let (token, _session) = svc.login("alice", "oldpassword1").await.unwrap();

        svc.admin_reset_password(user.id, "adminsetpassword1")
            .await
            .unwrap();

        let err = svc.validate_session(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidCredentials));
    }

    #[tokio::test]
    async fn admin_reset_password_weak_password() {
        let (pool, _dir) = test_pool().await;
        let svc = test_service(&pool);

        let user = svc.register("alice", "oldpassword1", None).await.unwrap();

        let err = svc
            .admin_reset_password(user.id, "short")
            .await
            .unwrap_err();

        assert!(matches!(err, AuthError::WeakPassword(_)));
    }
}
