use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Role assigned to a user account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Admin,
    User,
}

impl fmt::Display for UserRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admin => write!(f, "admin"),
            Self::User => write!(f, "user"),
        }
    }
}

impl FromStr for UserRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "admin" => Ok(Self::Admin),
            "user" => Ok(Self::User),
            other => Err(format!("unknown user role: {other}")),
        }
    }
}

/// A registered user account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub email: Option<String>,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
    pub is_active: bool,
}

impl User {
    /// Create a new active user with the given credentials and role.
    pub fn new(username: String, password_hash: String, role: UserRole) -> Self {
        Self {
            id: Uuid::new_v4(),
            username,
            email: None,
            password_hash,
            role,
            created_at: Utc::now(),
            is_active: true,
        }
    }
}

/// An authenticated session tied to a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl Session {
    /// Create a new session for the given user.
    pub fn new(user_id: Uuid, token_hash: String, expires_at: DateTime<Utc>) -> Self {
        Self {
            id: Uuid::new_v4(),
            user_id,
            token_hash,
            expires_at,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_role_display_and_parse() {
        assert_eq!(UserRole::Admin.to_string(), "admin");
        assert_eq!(UserRole::User.to_string(), "user");
        assert_eq!("admin".parse::<UserRole>().unwrap(), UserRole::Admin);
        assert_eq!("user".parse::<UserRole>().unwrap(), UserRole::User);
        assert!("bogus".parse::<UserRole>().is_err());
    }

    #[test]
    fn user_role_serde_roundtrip() {
        let role = UserRole::Admin;
        let json = serde_json::to_string(&role).unwrap();
        assert_eq!(json, r#""admin""#);
        let deserialized: UserRole = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, role);

        let role = UserRole::User;
        let json = serde_json::to_string(&role).unwrap();
        assert_eq!(json, r#""user""#);
        let deserialized: UserRole = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, role);
    }

    #[test]
    fn new_user_has_defaults() {
        let user = User::new(
            "testuser".to_string(),
            "hashed_pw".to_string(),
            UserRole::User,
        );
        assert!(!user.id.is_nil());
        assert_eq!(user.username, "testuser");
        assert!(user.email.is_none());
        assert_eq!(user.role, UserRole::User);
        assert!(user.is_active);
    }

    #[test]
    fn user_password_hash_not_serialized() {
        let user = User::new(
            "testuser".to_string(),
            "super_secret_hash".to_string(),
            UserRole::Admin,
        );
        let value = serde_json::to_value(&user).unwrap();
        assert!(
            value.get("password_hash").is_none(),
            "password_hash must not appear in serialized output"
        );
    }

    #[test]
    fn session_new_has_defaults() {
        let user_id = Uuid::new_v4();
        let expires = Utc::now() + chrono::Duration::hours(24);
        let session = Session::new(user_id, "token_hash_value".to_string(), expires);
        assert!(!session.id.is_nil());
        assert_eq!(session.user_id, user_id);
        assert_eq!(session.token_hash, "token_hash_value");
        assert_eq!(session.expires_at, expires);
    }
}
