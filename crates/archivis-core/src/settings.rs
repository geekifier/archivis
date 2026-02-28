/// Read-only access to configuration values.
///
/// Implemented by `ConfigService` in `archivis-api`; consumed by providers
/// and the resolver in `archivis-metadata` via `Arc<dyn SettingsReader>`.
/// This allows runtime settings changes (via the admin UI / API) to take
/// effect immediately without restarting the server.
pub trait SettingsReader: Send + Sync {
    /// Return the current value for `key`, or `None` if unset.
    fn get_setting(&self, key: &str) -> Option<serde_json::Value>;
}
