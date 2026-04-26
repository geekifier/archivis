//! Format transformer trait and registry.
//!
//! Transformers convert one in-memory ebook representation to another
//! (e.g. EPUB → KEPUB) on demand without persisting the output. They are
//! identified by a stable string id rather than extending [`BookFormat`],
//! because the targets are delivery-only formats that are never stored
//! or detected from disk.

use std::collections::HashMap;
use std::sync::Arc;

use archivis_core::errors::FormatError;
use archivis_core::models::BookFormat;
use tokio::sync::Semaphore;

/// A pluggable format transformer.
///
/// Implementations must be safe to call concurrently; the registry holds
/// transformers behind `Arc<dyn FormatTransformer>` and shares them across
/// requests.
pub trait FormatTransformer: Send + Sync {
    /// Stable, lowercase identifier (e.g. `"kepub"`).
    fn id(&self) -> &'static str;

    /// Semver string, bumped whenever transformer output changes in a way
    /// that should invalidate cached representations.
    fn version(&self) -> &'static str;

    /// Source format this transformer accepts.
    fn source_format(&self) -> BookFormat;

    /// Target MIME type emitted on success.
    fn target_mime(&self) -> &'static str;

    /// Filename extension for the transformed output (no leading dot).
    fn target_extension(&self) -> &'static str;

    /// Convert the input bytes into the target representation.
    ///
    /// Implementations must be deterministic: the same input must produce
    /// byte-identical output across calls and processes so HTTP `ETag`
    /// values stay stable.
    fn transform(&self, input: &[u8]) -> Result<Vec<u8>, FormatError>;
}

/// Registry of available [`FormatTransformer`] instances plus a shared
/// concurrency semaphore that callers acquire before running a transform.
#[derive(Clone)]
pub struct TransformerRegistry {
    by_target_id: Arc<HashMap<&'static str, Arc<dyn FormatTransformer>>>,
    permits: Arc<Semaphore>,
}

impl TransformerRegistry {
    /// Build a registry with the supplied transformers and a default permit
    /// count of `min(num_cpus, 4).max(2)`.
    #[must_use]
    pub fn new(transformers: Vec<Arc<dyn FormatTransformer>>) -> Self {
        let permit_count = default_permit_count();
        Self::with_permits(transformers, permit_count)
    }

    /// Build a registry with an explicit permit count. Useful for tests.
    #[must_use]
    pub fn with_permits(transformers: Vec<Arc<dyn FormatTransformer>>, permits: usize) -> Self {
        let mut by_target_id: HashMap<&'static str, Arc<dyn FormatTransformer>> = HashMap::new();
        for t in transformers {
            by_target_id.insert(t.id(), t);
        }
        Self {
            by_target_id: Arc::new(by_target_id),
            permits: Arc::new(Semaphore::new(permits.max(1))),
        }
    }

    /// Build an empty registry. Used by tests that do not exercise the
    /// transform path; production code constructs registries through
    /// [`Self::new`].
    #[must_use]
    pub fn empty() -> Self {
        Self::with_permits(Vec::new(), default_permit_count())
    }

    /// Look up a transformer by its target id (e.g. `"kepub"`).
    #[must_use]
    pub fn lookup(&self, target_id: &str) -> Option<Arc<dyn FormatTransformer>> {
        self.by_target_id.get(target_id).map(Arc::clone)
    }

    /// Iterate the registered target ids. Order is unspecified.
    #[must_use]
    pub fn known_target_ids(&self) -> Vec<&'static str> {
        self.by_target_id.keys().copied().collect()
    }

    /// Return the shared concurrency semaphore. Callers acquire one permit
    /// before invoking [`FormatTransformer::transform`].
    #[must_use]
    pub fn permits(&self) -> Arc<Semaphore> {
        Arc::clone(&self.permits)
    }
}

impl Default for TransformerRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

fn default_permit_count() -> usize {
    let cpus = std::thread::available_parallelism().map_or(2, std::num::NonZeroUsize::get);
    cpus.clamp(2, 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyTransformer;

    impl FormatTransformer for DummyTransformer {
        fn id(&self) -> &'static str {
            "dummy"
        }
        fn version(&self) -> &'static str {
            "0.0.1"
        }
        fn source_format(&self) -> BookFormat {
            BookFormat::Epub
        }
        fn target_mime(&self) -> &'static str {
            "application/octet-stream"
        }
        fn target_extension(&self) -> &'static str {
            "bin"
        }
        fn transform(&self, input: &[u8]) -> Result<Vec<u8>, FormatError> {
            Ok(input.to_vec())
        }
    }

    #[test]
    fn lookup_returns_registered_transformer() {
        let registry = TransformerRegistry::new(vec![Arc::new(DummyTransformer)]);
        let t = registry.lookup("dummy").expect("dummy registered");
        assert_eq!(t.id(), "dummy");
        assert_eq!(t.target_mime(), "application/octet-stream");
    }

    #[test]
    fn empty_registry_has_no_transformers() {
        let registry = TransformerRegistry::empty();
        assert!(registry.lookup("kepub").is_none());
    }

    #[tokio::test]
    async fn permits_can_be_acquired() {
        let registry = TransformerRegistry::with_permits(Vec::new(), 2);
        let permits = registry.permits();
        let p1 = permits.clone().acquire_owned().await.unwrap();
        let p2 = permits.acquire_owned().await.unwrap();
        drop((p1, p2));
    }
}
