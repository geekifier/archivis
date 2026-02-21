use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use sqlx::SqlitePool;

use archivis_core::errors::{AuthError, DbError};
use archivis_core::models::{User, UserRole};
use archivis_db::UserRepository;

use crate::adapter::AuthAdapter;

/// Local authentication adapter using Argon2id password hashing.
pub struct LocalAuthAdapter {
    pool: SqlitePool,
}

impl LocalAuthAdapter {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn hash_password(password: &str) -> Result<String, AuthError> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| AuthError::Internal(format!("password hashing failed: {e}")))?;
        Ok(hash.to_string())
    }

    fn verify_password(password: &str, hash: &str) -> Result<bool, AuthError> {
        let parsed_hash = PasswordHash::new(hash)
            .map_err(|e| AuthError::Internal(format!("invalid password hash format: {e}")))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }

    fn validate_password(password: &str) -> Result<(), AuthError> {
        if password.len() < 8 {
            return Err(AuthError::WeakPassword(
                "password must be at least 8 characters".into(),
            ));
        }
        Ok(())
    }
}

impl AuthAdapter for LocalAuthAdapter {
    async fn authenticate(&self, username: &str, password: &str) -> Result<User, AuthError> {
        let user = UserRepository::get_by_username(&self.pool, username)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => AuthError::InvalidCredentials,
                other => AuthError::Internal(other.to_string()),
            })?;

        if !user.is_active {
            return Err(AuthError::Unauthorized);
        }

        // Verify in blocking task to avoid blocking the async runtime
        let hash = user.password_hash.clone();
        let pwd = password.to_string();
        let valid = tokio::task::spawn_blocking(move || Self::verify_password(&pwd, &hash))
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))??;

        if !valid {
            return Err(AuthError::InvalidCredentials);
        }

        Ok(user)
    }

    async fn register(
        &self,
        username: &str,
        password: &str,
        email: Option<&str>,
    ) -> Result<User, AuthError> {
        Self::validate_password(password)?;

        // Check if username already exists
        match UserRepository::get_by_username(&self.pool, username).await {
            Ok(_) => return Err(AuthError::UserExists(username.to_string())),
            Err(DbError::NotFound { .. }) => {} // Username is available
            Err(e) => return Err(AuthError::Internal(e.to_string())),
        }

        // Hash password in blocking task
        let pwd = password.to_string();
        let password_hash = tokio::task::spawn_blocking(move || Self::hash_password(&pwd))
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))??;

        // First user gets Admin role
        let count = UserRepository::count(&self.pool)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        let role = if count == 0 {
            UserRole::Admin
        } else {
            UserRole::User
        };

        let mut user = User::new(username.to_string(), password_hash, role);
        user.email = email.map(ToString::to_string);

        UserRepository::create(&self.pool, &user)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        Ok(user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_password() {
        let hash = LocalAuthAdapter::hash_password("correct_horse_battery").unwrap();
        assert!(LocalAuthAdapter::verify_password("correct_horse_battery", &hash).unwrap());
    }

    #[test]
    fn verify_wrong_password() {
        let hash = LocalAuthAdapter::hash_password("correct_horse_battery").unwrap();
        assert!(!LocalAuthAdapter::verify_password("wrong_password", &hash).unwrap());
    }

    #[test]
    fn validate_password_too_short() {
        let result = LocalAuthAdapter::validate_password("short");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AuthError::WeakPassword(_)));
    }

    #[test]
    fn validate_password_ok() {
        assert!(LocalAuthAdapter::validate_password("long_enough_password").is_ok());
    }
}
