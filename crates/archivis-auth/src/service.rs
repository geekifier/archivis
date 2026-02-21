use chrono::Utc;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use archivis_core::errors::{AuthError, DbError};
use archivis_core::models::{Session, User};
use archivis_db::SessionRepository;

use crate::adapter::AuthAdapter;

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

        let user = archivis_db::UserRepository::get_by_id(&self.pool, session.user_id)
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
        let count = archivis_db::UserRepository::count(&self.pool)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        Ok(count == 0)
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
}
