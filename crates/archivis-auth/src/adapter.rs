use archivis_core::errors::AuthError;
use archivis_core::models::User;

/// Pluggable authentication backend.
///
/// Only `LocalAuthAdapter` is implemented for MVP. The trait enables
/// OAuth2/OIDC and reverse-proxy auth adapters in the future.
pub trait AuthAdapter: Send + Sync {
    /// Verify credentials and return the authenticated user.
    fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> impl std::future::Future<Output = Result<User, AuthError>> + Send;

    /// Register a new user account.
    fn register(
        &self,
        username: &str,
        password: &str,
        email: Option<&str>,
    ) -> impl std::future::Future<Output = Result<User, AuthError>> + Send;
}
