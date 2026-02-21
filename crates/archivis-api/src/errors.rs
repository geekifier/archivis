use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use archivis_core::errors::{ArchivisError, AuthError, DbError, StorageError};

/// API-layer error type that maps domain errors to HTTP responses.
///
/// Handlers return `Result<T, ApiError>` and Axum automatically converts
/// errors to JSON responses with appropriate status codes.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// A domain error propagated from the service layer.
    #[error(transparent)]
    Core(#[from] ArchivisError),

    /// Request validation failed (malformed input, missing fields, etc.).
    #[error("validation error: {0}")]
    Validation(String),

    /// The requested resource was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// Authentication required or credentials invalid.
    #[error("unauthorized")]
    Unauthorized,

    /// Authenticated but lacking permissions.
    #[error("forbidden")]
    Forbidden,

    /// An unexpected internal error.
    #[error("internal server error")]
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            Self::Forbidden => (StatusCode::FORBIDDEN, self.to_string()),
            Self::Internal(msg) => {
                tracing::error!(error = %msg, "internal server error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".into(),
                )
            }
            Self::Core(err) => map_core_error(err),
        };

        let body = serde_json::json!({
            "error": {
                "status": status.as_u16(),
                "message": message,
            }
        });

        (status, axum::Json(body)).into_response()
    }
}

/// Map a domain error to an HTTP status code and user-facing message.
///
/// Internal details are logged but not exposed to the client.
fn map_core_error(err: &ArchivisError) -> (StatusCode, String) {
    match err {
        ArchivisError::Db(DbError::NotFound { entity, id }) => {
            (StatusCode::NOT_FOUND, format!("{entity} not found: {id}"))
        }
        ArchivisError::Db(db_err) => {
            tracing::error!(error = %db_err, "database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        }
        ArchivisError::Auth(
            AuthError::InvalidCredentials | AuthError::SessionExpired | AuthError::Unauthorized,
        ) => (StatusCode::UNAUTHORIZED, err.to_string()),
        ArchivisError::Auth(AuthError::Forbidden) => (StatusCode::FORBIDDEN, err.to_string()),
        ArchivisError::Auth(AuthError::UserExists(name)) => {
            (StatusCode::CONFLICT, format!("user already exists: {name}"))
        }
        ArchivisError::Auth(AuthError::WeakPassword(msg)) => (StatusCode::BAD_REQUEST, msg.clone()),
        ArchivisError::Auth(auth_err) => {
            tracing::error!(error = %auth_err, "auth error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        }
        ArchivisError::Format(_) | ArchivisError::Metadata(_) => {
            (StatusCode::UNPROCESSABLE_ENTITY, err.to_string())
        }
        ArchivisError::Storage(storage_err) => {
            tracing::error!(error = %storage_err, "storage error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        }
        ArchivisError::Task(task_err) => {
            tracing::error!(error = %task_err, "task error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        }
    }
}

// Convenience From impls so handlers can use `?` on per-crate errors directly.
impl From<DbError> for ApiError {
    fn from(err: DbError) -> Self {
        Self::Core(ArchivisError::from(err))
    }
}

impl From<AuthError> for ApiError {
    fn from(err: AuthError) -> Self {
        Self::Core(ArchivisError::from(err))
    }
}

impl From<StorageError> for ApiError {
    fn from(err: StorageError) -> Self {
        Self::Core(ArchivisError::from(err))
    }
}

impl From<validator::ValidationErrors> for ApiError {
    fn from(err: validator::ValidationErrors) -> Self {
        Self::Validation(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    async fn response_status_and_body(err: ApiError) -> (StatusCode, serde_json::Value) {
        let response = err.into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        (status, json)
    }

    #[tokio::test]
    async fn validation_error_returns_400() {
        let (status, body) =
            response_status_and_body(ApiError::Validation("title is required".into())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["status"], 400);
        assert_eq!(body["error"]["message"], "title is required");
    }

    #[tokio::test]
    async fn not_found_returns_404() {
        let (status, body) =
            response_status_and_body(ApiError::NotFound("book not found".into())).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["status"], 404);
    }

    #[tokio::test]
    async fn unauthorized_returns_401() {
        let (status, _) = response_status_and_body(ApiError::Unauthorized).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn forbidden_returns_403() {
        let (status, _) = response_status_and_body(ApiError::Forbidden).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn db_not_found_returns_404() {
        let err = ApiError::from(DbError::NotFound {
            entity: "book",
            id: "abc-123".into(),
        });
        let (status, body) = response_status_and_body(err).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["message"], "book not found: abc-123");
    }

    #[tokio::test]
    async fn db_connection_error_returns_500() {
        let err = ApiError::from(DbError::Connection("timeout".into()));
        let (status, body) = response_status_and_body(err).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        // Internal details not exposed
        assert_eq!(body["error"]["message"], "internal server error");
    }

    #[tokio::test]
    async fn auth_invalid_credentials_returns_401() {
        let err = ApiError::from(AuthError::InvalidCredentials);
        let (status, _) = response_status_and_body(err).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_forbidden_returns_403() {
        let err = ApiError::from(AuthError::Forbidden);
        let (status, _) = response_status_and_body(err).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn user_exists_returns_409() {
        let err = ApiError::from(AuthError::UserExists("admin".into()));
        let (status, _) = response_status_and_body(err).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn internal_error_hides_details() {
        let (status, body) =
            response_status_and_body(ApiError::Internal("secret db info".into())).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"]["message"], "internal server error");
    }
}
