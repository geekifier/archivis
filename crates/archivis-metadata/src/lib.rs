pub mod client;
pub mod errors;
pub mod provider;
pub mod providers;
pub mod registry;
pub mod resolver;
pub mod similarity;
pub mod types;

pub use client::MetadataHttpClient;
pub use errors::ProviderError;
pub use provider::MetadataProvider;
pub use providers::HardcoverProvider;
pub use providers::OpenLibraryProvider;
pub use registry::ProviderRegistry;
pub use resolver::{
    CandidateMatchTier, ExistingBookMetadata, MetadataResolver, ResolverDecision, ResolverResult,
    ScoredCandidate,
};
pub use types::{
    MetadataQuery, ProviderAuthor, ProviderIdentifier, ProviderMetadata, ProviderSeries,
};

#[cfg(test)]
pub(crate) mod test_util {
    use std::collections::HashMap;

    use archivis_core::settings::SettingsReader;

    /// In-memory stub for unit tests that need a `SettingsReader`.
    pub struct StubSettings(pub HashMap<String, serde_json::Value>);

    impl StubSettings {
        pub fn new(entries: Vec<(&str, serde_json::Value)>) -> Self {
            Self(
                entries
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            )
        }
    }

    impl SettingsReader for StubSettings {
        fn get_setting(&self, key: &str) -> Option<serde_json::Value> {
            self.0.get(key).cloned()
        }
    }
}
