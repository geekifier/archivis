mod backend;

pub mod local;
pub mod path;

pub use backend::{FileMetadata, StorageBackend, StoredFile};
