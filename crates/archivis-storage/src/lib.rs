mod backend;

pub mod local;
pub mod path;
pub mod watcher;

pub use backend::{FileMetadata, StorageBackend, StoredFile};
