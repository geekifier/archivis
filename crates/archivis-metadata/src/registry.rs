use std::sync::Arc;

use crate::provider::MetadataProvider;
use crate::types::ProviderCapabilities;

/// Registry of all configured metadata providers.
///
/// Holds references to every provider (enabled or not) and provides
/// methods to query only those that are currently available.
pub struct ProviderRegistry {
    providers: Vec<Arc<dyn MetadataProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Add a provider to the registry.
    pub fn register(&mut self, provider: Arc<dyn MetadataProvider>) {
        self.providers.push(provider);
    }

    /// Returns all available (enabled + configured) providers.
    pub fn available(&self) -> Vec<Arc<dyn MetadataProvider>> {
        self.providers
            .iter()
            .filter(|p| p.is_available())
            .cloned()
            .collect()
    }

    /// Get a specific provider by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn MetadataProvider>> {
        self.providers.iter().find(|p| p.name() == name).cloned()
    }

    /// Returns `(provider_name, capabilities)` for every registered provider.
    pub fn all_capabilities(&self) -> Vec<(&str, &'static ProviderCapabilities)> {
        self.providers
            .iter()
            .map(|p| (p.name(), p.capabilities()))
            .collect()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use archivis_core::models::IdentifierType;

    use super::*;
    use crate::errors::ProviderError;
    use crate::types::{MetadataQuery, ProviderFeature, ProviderMetadata, ProviderQuality};

    static STUB_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
        quality: ProviderQuality::Community,
        default_rate_limit_rpm: 100,
        supported_id_lookups: &[IdentifierType::Isbn13, IdentifierType::Isbn10],
        features: &[ProviderFeature::Search, ProviderFeature::Covers],
    };

    /// Minimal stub provider for testing the registry.
    struct StubProvider {
        name: &'static str,
        available: bool,
    }

    #[async_trait]
    impl MetadataProvider for StubProvider {
        fn name(&self) -> &str {
            self.name
        }

        fn is_available(&self) -> bool {
            self.available
        }

        fn capabilities(&self) -> &'static ProviderCapabilities {
            &STUB_CAPABILITIES
        }

        async fn lookup_isbn(&self, _isbn: &str) -> Result<Vec<ProviderMetadata>, ProviderError> {
            Ok(vec![])
        }

        async fn search(
            &self,
            _query: &MetadataQuery,
        ) -> Result<Vec<ProviderMetadata>, ProviderError> {
            Ok(vec![])
        }

        async fn fetch_cover(&self, _cover_url: &str) -> Result<Vec<u8>, ProviderError> {
            Ok(vec![])
        }
    }

    #[test]
    fn empty_registry_returns_no_providers() {
        let registry = ProviderRegistry::new();
        assert!(registry.available().is_empty());
        assert!(registry.get("anything").is_none());
    }

    #[test]
    fn available_returns_only_available_providers() {
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(StubProvider {
            name: "enabled_one",
            available: true,
        }));
        registry.register(Arc::new(StubProvider {
            name: "disabled_one",
            available: false,
        }));
        registry.register(Arc::new(StubProvider {
            name: "enabled_two",
            available: true,
        }));

        let available = registry.available();
        assert_eq!(available.len(), 2);
        assert_eq!(available[0].name(), "enabled_one");
        assert_eq!(available[1].name(), "enabled_two");
    }

    #[test]
    fn get_finds_provider_by_name() {
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(StubProvider {
            name: "open_library",
            available: true,
        }));
        registry.register(Arc::new(StubProvider {
            name: "hardcover",
            available: false,
        }));

        assert!(registry.get("open_library").is_some());
        assert!(registry.get("hardcover").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn get_returns_disabled_providers_too() {
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(StubProvider {
            name: "hardcover",
            available: false,
        }));

        // get() returns any registered provider regardless of availability
        let provider = registry.get("hardcover");
        assert!(provider.is_some());
        assert!(!provider.unwrap().is_available());
    }

    #[test]
    fn all_capabilities_returns_all_registered() {
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(StubProvider {
            name: "open_library",
            available: true,
        }));
        registry.register(Arc::new(StubProvider {
            name: "hardcover",
            available: false,
        }));

        let caps = registry.all_capabilities();
        assert_eq!(caps.len(), 2);
        assert_eq!(caps[0].0, "open_library");
        assert_eq!(caps[1].0, "hardcover");
        assert_eq!(caps[0].1.quality, ProviderQuality::Community);
    }
}
