use serde::Deserialize;
use utoipa::ToSchema;
use validator::Validate;

use archivis_core::models::UserRole;

/// Request body for `POST /api/users`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateUserRequest {
    #[validate(length(min = 1, message = "username must not be empty"))]
    pub username: String,
    #[validate(length(min = 8, message = "password must be at least 8 characters"))]
    pub password: String,
    #[validate(email)]
    pub email: Option<String>,
    #[schema(value_type = String)]
    pub role: UserRole,
}

/// Request body for `PUT /api/users/:id`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateUserRequest {
    #[validate(length(min = 1, message = "username must not be empty"))]
    pub username: Option<String>,
    pub email: Option<Option<String>>,
    #[schema(value_type = Option<String>)]
    pub role: Option<UserRole>,
    pub is_active: Option<bool>,
}

/// Request body for `PUT /api/users/:id/password`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct AdminResetPasswordRequest {
    #[validate(length(min = 8, message = "password must be at least 8 characters"))]
    pub new_password: String,
}

/// Request body for `PUT /api/auth/password`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ChangePasswordRequest {
    #[validate(length(min = 1, message = "current password must not be empty"))]
    pub current_password: String,
    #[validate(length(min = 8, message = "new password must be at least 8 characters"))]
    pub new_password: String,
}
