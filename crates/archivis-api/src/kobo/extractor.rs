use std::fmt::Write;

use axum::extract::{FromRequestParts, OriginalUri, Request};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use archivis_core::errors::DbError;
use archivis_core::models::{KoboDevice, User};
use archivis_db::{KoboDeviceRepository, UserRepository};

use crate::state::AppState;

/// Raw token captured by the [`kobo_token_layer`] middleware.
#[derive(Debug, Clone)]
pub struct KoboToken(pub String);

/// Middleware: parse the first path segment after `/kobo/` from the
/// `OriginalUri` and stash it as a [`KoboToken`] extension.
///
/// We reach for `OriginalUri` so this works correctly when the router
/// `nest`s at `/kobo` — `req.uri()` would be `/v1/...` inside the nest.
pub async fn kobo_token_layer(mut req: Request, next: Next) -> Response {
    let path = req
        .extensions()
        .get::<OriginalUri>()
        .map_or_else(|| req.uri().path().to_string(), |o| o.0.path().to_string());

    let Some(token) = parse_kobo_token(&path) else {
        return reject(StatusCode::UNAUTHORIZED, "missing kobo token");
    };

    req.extensions_mut().insert(KoboToken(token));
    next.run(req).await
}

fn parse_kobo_token(path: &str) -> Option<String> {
    let trimmed = path.trim_start_matches('/');
    let after_kobo = trimmed.strip_prefix("kobo/")?;
    let token = after_kobo.split('/').next()?;
    if token.is_empty() {
        return None;
    }
    Some(token.to_string())
}

fn reject(status: StatusCode, message: &str) -> Response {
    let body = serde_json::json!({"error": {"status": status.as_u16(), "message": message}});
    (status, axum::Json(body)).into_response()
}

/// Hash a Kobo pairing token with SHA-256, returning lowercase hex.
pub fn hash_kobo_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let result = hasher.finalize();
    let mut s = String::with_capacity(result.len() * 2);
    for b in result {
        write!(s, "{b:02x}").expect("hex encoding cannot fail");
    }
    s
}

/// Generate a 32-byte opaque pairing token, hex-encoded.
pub fn generate_kobo_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(s, "{b:02x}").expect("hex encoding cannot fail");
    }
    s
}

/// Authenticated Kobo device + the owning Archivis user.
pub struct KoboDeviceAuth {
    pub device: KoboDevice,
    pub user: User,
    /// Raw pairing token as supplied by the device — used to re-emit URLs
    /// in protocol responses. Not stored anywhere persistently.
    pub raw_token: String,
}

#[derive(Debug)]
pub struct KoboAuthRejection(StatusCode, String);

impl IntoResponse for KoboAuthRejection {
    fn into_response(self) -> Response {
        reject(self.0, &self.1)
    }
}

impl FromRequestParts<AppState> for KoboDeviceAuth {
    type Rejection = KoboAuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .extensions
            .get::<KoboToken>()
            .map(|t| t.0.clone())
            .ok_or_else(|| {
                KoboAuthRejection(StatusCode::UNAUTHORIZED, "missing kobo token".into())
            })?;

        let token_hash = hash_kobo_token(&token);
        let pool = state.db_pool();

        let device = match KoboDeviceRepository::get_active_by_token_hash(pool, &token_hash).await {
            Ok(d) => d,
            Err(DbError::NotFound { .. }) => {
                return Err(KoboAuthRejection(
                    StatusCode::UNAUTHORIZED,
                    "invalid kobo token".into(),
                ));
            }
            Err(e) => {
                tracing::error!(error = %e, "kobo device lookup failed");
                return Err(KoboAuthRejection(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".into(),
                ));
            }
        };

        let user = UserRepository::get_by_id(pool, device.user_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "kobo user lookup failed");
                KoboAuthRejection(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".into(),
                )
            })?;

        if !user.is_active {
            return Err(KoboAuthRejection(
                StatusCode::UNAUTHORIZED,
                "user inactive".into(),
            ));
        }

        // Best-effort last_seen update — never fail the request on this.
        if let Err(e) = KoboDeviceRepository::touch_last_seen(pool, device.id).await {
            tracing::debug!(error = %e, device_id = %device.id, "failed to update last_seen_at");
        }

        Ok(Self {
            device,
            user,
            raw_token: token,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_token_from_root_path() {
        assert_eq!(
            parse_kobo_token("/kobo/abc123/v1/library/sync"),
            Some("abc123".into())
        );
    }

    #[test]
    fn parses_token_at_segment_end() {
        assert_eq!(parse_kobo_token("/kobo/abc123"), Some("abc123".into()));
    }

    #[test]
    fn rejects_missing_token() {
        assert_eq!(parse_kobo_token("/kobo/"), None);
        assert_eq!(parse_kobo_token("/other/abc"), None);
    }

    #[test]
    fn hashed_tokens_are_deterministic_hex() {
        let a = hash_kobo_token("token-abc");
        let b = hash_kobo_token("token-abc");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generated_tokens_are_unique_64_hex() {
        let a = generate_kobo_token();
        let b = generate_kobo_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
