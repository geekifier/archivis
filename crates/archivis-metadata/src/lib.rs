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
pub use resolver::{ExistingBookMetadata, MetadataResolver, ResolverResult, ScoredCandidate};
pub use types::{
    MetadataQuery, ProviderAuthor, ProviderIdentifier, ProviderMetadata, ProviderSeries,
};
