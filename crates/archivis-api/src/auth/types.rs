use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use archivis_core::models::{User, UserRole};

/// Request body for initial admin setup.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct SetupRequest {
    #[validate(length(min = 1, message = "username must not be empty"))]
    pub username: String,
    #[validate(length(min = 8, message = "password must be at least 8 characters"))]
    pub password: String,
    #[validate(email)]
    pub email: Option<String>,
}

/// Request body for login.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct LoginRequest {
    #[validate(length(min = 1))]
    pub username: String,
    #[validate(length(min = 1))]
    pub password: String,
}

/// Response for GET /api/auth/status.
#[derive(Debug, Serialize, ToSchema)]
pub struct AuthStatusResponse {
    pub setup_required: bool,
}

/// User data returned in API responses (`password_hash` excluded).
#[derive(Debug, Serialize, ToSchema)]
pub struct UserResponse {
    pub id: Uuid,
    pub username: String,
    pub email: Option<String>,
    #[schema(value_type = String)]
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
    pub is_active: bool,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            username: user.username,
            email: user.email,
            role: user.role,
            created_at: user.created_at,
            is_active: user.is_active,
        }
    }
}

/// Response for POST /api/auth/login.
#[derive(Debug, Serialize, ToSchema)]
pub struct LoginResponse {
    pub token: String,
    pub user: UserResponse,
}
