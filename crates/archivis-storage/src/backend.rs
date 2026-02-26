use archivis_core::errors::StorageError;
use chrono::{DateTime, Utc};

/// Result of a successful file store operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    /// Relative path within the storage backend.
    pub path: String,
    /// File size in bytes.
    pub size: u64,
    /// SHA-256 hash of the file contents, hex-encoded.
    pub hash: String,
}

/// Filesystem-level metadata about a stored file.
#[derive(Debug, Clone)]
pub struct FileMetadata {
    /// File size in bytes.
    pub size: u64,
    /// Last modified time.
    pub modified: DateTime<Utc>,
    /// Creation time, if available.
    pub created: Option<DateTime<Utc>>,
}

/// Abstraction over file storage backends.
///
/// All paths are relative to the storage root. The backend resolves them
/// against its configured root directory.
pub trait StorageBackend: Send + Sync {
    /// Store data at the given relative path.
    ///
    /// Creates parent directories as needed. Computes SHA-256 hash during
    /// write and returns it in the [`StoredFile`].
    fn store(
        &self,
        path: &str,
        data: &[u8],
    ) -> impl std::future::Future<Output = Result<StoredFile, StorageError>> + Send;

    /// Store data with a pre-computed SHA-256 hash, avoiding redundant hashing.
    ///
    /// Callers that have already hashed the data (e.g. for duplicate detection)
    /// can pass the hash here to skip re-computation during the write.
    fn store_with_hash(
        &self,
        path: &str,
        data: &[u8],
        hash: String,
    ) -> impl std::future::Future<Output = Result<StoredFile, StorageError>> + Send;

    /// Read the entire contents of a stored file.
    fn read(
        &self,
        path: &str,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, StorageError>> + Send;

    /// Delete a stored file.
    ///
    /// Cleans up empty parent directories up to the storage root.
    fn delete(
        &self,
        path: &str,
    ) -> impl std::future::Future<Output = Result<(), StorageError>> + Send;

    /// Check whether a file exists at the given path.
    fn exists(
        &self,
        path: &str,
    ) -> impl std::future::Future<Output = Result<bool, StorageError>> + Send;

    /// Get filesystem-level metadata for a stored file.
    fn file_metadata(
        &self,
        path: &str,
    ) -> impl std::future::Future<Output = Result<FileMetadata, StorageError>> + Send;
}
