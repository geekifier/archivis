use async_trait::async_trait;

use crate::errors::ProviderError;
use crate::types::{MetadataQuery, ProviderMetadata};

/// Trait that all metadata providers must implement.
///
/// Each provider connects to an external metadata source (Open Library,
/// Hardcover, etc.) and returns structured metadata for books.
#[async_trait]
pub trait MetadataProvider: Send + Sync {
    /// Provider identifier, e.g. `"open_library"`, `"hardcover"`.
    fn name(&self) -> &str;

    /// Whether this provider is enabled and configured.
    fn is_available(&self) -> bool;

    /// Look up a book by ISBN. Returns candidates sorted by confidence
    /// (highest first).
    async fn lookup_isbn(&self, isbn: &str) -> Result<Vec<ProviderMetadata>, ProviderError>;

    /// Search by title and/or author. Returns candidates sorted by relevance.
    async fn search(&self, query: &MetadataQuery) -> Result<Vec<ProviderMetadata>, ProviderError>;

    /// Fetch cover image bytes from a provider-specific URL.
    async fn fetch_cover(&self, cover_url: &str) -> Result<Vec<u8>, ProviderError>;
}
