use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use tokio::fs;
use tracing::debug;

use archivis_core::errors::StorageError;

use crate::{FileMetadata, StorageBackend, StoredFile};

/// Local filesystem storage backend.
///
/// Stores files under a configurable root directory. All paths passed to trait
/// methods are relative and resolved against this root. Path traversal attempts
/// (e.g. `../`) are rejected.
#[derive(Debug, Clone)]
pub struct LocalStorage {
    root: PathBuf,
}

impl LocalStorage {
    /// Create a new `LocalStorage` rooted at the given directory.
    ///
    /// Creates the root directory (and parents) if it doesn't exist.
    /// The path is canonicalized to resolve symlinks.
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let root = root.into();
        fs::create_dir_all(&root).await?;
        let root = root
            .canonicalize()
            .map_err(|e| StorageError::Io { source: e })?;
        Ok(Self { root })
    }

    /// Return the absolute root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a relative storage path to an absolute filesystem path.
    ///
    /// Rejects absolute paths, null bytes, and path traversal attempts.
    fn resolve(&self, relative_path: &str) -> Result<PathBuf, StorageError> {
        if relative_path.contains('\0') {
            return Err(StorageError::PathConflict("null byte in path".to_owned()));
        }

        let rel = Path::new(relative_path);
        if rel.is_absolute() {
            return Err(StorageError::PathConflict(
                "absolute paths not allowed".to_owned(),
            ));
        }

        let joined = self.root.join(rel);
        let normalized = normalize_path(&joined);

        if !normalized.starts_with(&self.root) {
            return Err(StorageError::PathConflict(format!(
                "path traversal detected: {relative_path}"
            )));
        }

        Ok(normalized)
    }
}

impl StorageBackend for LocalStorage {
    async fn store(&self, path: &str, data: &[u8]) -> Result<StoredFile, StorageError> {
        let full_path = self.resolve(path)?;

        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let hash = sha256_hex(data);

        // Write via temp file + rename for crash safety.
        let temp_path = sibling_temp_path(&full_path);
        let write_result = async {
            fs::write(&temp_path, data).await?;
            fs::rename(&temp_path, &full_path).await
        }
        .await;

        if let Err(e) = write_result {
            let _ = fs::remove_file(&temp_path).await;
            return Err(StorageError::Io { source: e });
        }

        debug!(path, size = data.len(), %hash, "stored file");

        Ok(StoredFile {
            path: path.to_owned(),
            size: data.len() as u64,
            hash,
        })
    }

    async fn read(&self, path: &str) -> Result<Vec<u8>, StorageError> {
        let full_path = self.resolve(path)?;
        fs::read(&full_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StorageError::NotFound(path.to_owned())
            } else {
                StorageError::Io { source: e }
            }
        })
    }

    async fn delete(&self, path: &str) -> Result<(), StorageError> {
        let full_path = self.resolve(path)?;
        fs::remove_file(&full_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StorageError::NotFound(path.to_owned())
            } else {
                StorageError::Io { source: e }
            }
        })?;

        // Clean up empty parent directories up to the storage root.
        self.prune_empty_parents(&full_path).await;

        debug!(path, "deleted file");
        Ok(())
    }

    async fn exists(&self, path: &str) -> Result<bool, StorageError> {
        let full_path = self.resolve(path)?;
        match fs::metadata(&full_path).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(StorageError::Io { source: e }),
        }
    }

    async fn file_metadata(&self, path: &str) -> Result<FileMetadata, StorageError> {
        let full_path = self.resolve(path)?;
        let meta = fs::metadata(&full_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StorageError::NotFound(path.to_owned())
            } else {
                StorageError::Io { source: e }
            }
        })?;

        let modified = meta
            .modified()
            .map(DateTime::<Utc>::from)
            .map_err(|e| StorageError::Io { source: e })?;

        let created = meta.created().map(DateTime::<Utc>::from).ok();

        Ok(FileMetadata {
            size: meta.len(),
            modified,
            created,
        })
    }
}

impl LocalStorage {
    /// Remove empty parent directories from `path` up to (not including) the
    /// storage root. Stops at the first non-empty directory.
    async fn prune_empty_parents(&self, path: &Path) {
        let mut current = path.parent().map(Path::to_path_buf);
        while let Some(dir) = current {
            if dir == self.root {
                break;
            }
            if fs::remove_dir(&dir).await.is_err() {
                break;
            }
            current = dir.parent().map(Path::to_path_buf);
        }
    }
}

/// Compute SHA-256 of `data` and return the hex-encoded digest.
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();

    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Generate a temp file path alongside `target` for atomic writes.
///
/// Example: `/a/b/file.epub` → `/a/b/.file.epub.tmp`
fn sibling_temp_path(target: &Path) -> PathBuf {
    let file_name = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!(".{file_name}.tmp"))
}

/// Normalize a path by resolving `.` and `..` components lexically, without
/// touching the filesystem (unlike [`Path::canonicalize`]).
fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            other => result.push(other),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup() -> (TempDir, LocalStorage) {
        let dir = TempDir::new().expect("create temp dir");
        let storage = LocalStorage::new(dir.path()).await.expect("create storage");
        (dir, storage)
    }

    #[tokio::test]
    async fn store_and_read_roundtrip() {
        let (_dir, storage) = setup().await;
        let data = b"Hello, Archivis!";
        let path = "test/hello.txt";

        let stored = storage.store(path, data).await.unwrap();
        assert_eq!(stored.path, path);
        assert_eq!(stored.size, data.len() as u64);
        assert!(!stored.hash.is_empty());

        let read_data = storage.read(path).await.unwrap();
        assert_eq!(read_data, data);
    }

    #[tokio::test]
    async fn store_computes_correct_sha256() {
        let (_dir, storage) = setup().await;
        let data = b"test data for hashing";
        let expected_hash = sha256_hex(data);

        let stored = storage.store("hash_test.bin", data).await.unwrap();
        assert_eq!(stored.hash, expected_hash);
    }

    #[tokio::test]
    async fn store_creates_parent_directories() {
        let (_dir, storage) = setup().await;
        let path = "deep/nested/dir/structure/file.epub";

        storage.store(path, b"epub content").await.unwrap();
        assert!(storage.exists(path).await.unwrap());
    }

    #[tokio::test]
    async fn store_overwrites_existing_file() {
        let (_dir, storage) = setup().await;
        let path = "overwrite.txt";

        storage.store(path, b"original").await.unwrap();
        storage.store(path, b"replaced").await.unwrap();

        let data = storage.read(path).await.unwrap();
        assert_eq!(data, b"replaced");
    }

    #[tokio::test]
    async fn delete_removes_file() {
        let (_dir, storage) = setup().await;
        let path = "to_delete.txt";

        storage.store(path, b"delete me").await.unwrap();
        assert!(storage.exists(path).await.unwrap());

        storage.delete(path).await.unwrap();
        assert!(!storage.exists(path).await.unwrap());
    }

    #[tokio::test]
    async fn delete_cleans_up_empty_parents() {
        let (_dir, storage) = setup().await;
        let path = "A/Author Name/Book Title/book.epub";

        storage.store(path, b"content").await.unwrap();
        storage.delete(path).await.unwrap();

        // All intermediate dirs should be removed since they're empty
        assert!(!storage.root().join("A").exists());
    }

    #[tokio::test]
    async fn delete_preserves_non_empty_parents() {
        let (_dir, storage) = setup().await;

        storage
            .store("A/Author/Book1/a.epub", b"book1")
            .await
            .unwrap();
        storage
            .store("A/Author/Book2/b.epub", b"book2")
            .await
            .unwrap();

        storage.delete("A/Author/Book1/a.epub").await.unwrap();

        // "A/Author/" still has "Book2/" so should not be removed
        assert!(storage.exists("A/Author/Book2/b.epub").await.unwrap());
        assert!(storage.root().join("A/Author").exists());
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_not_found() {
        let (_dir, storage) = setup().await;
        let err = storage.delete("nonexistent.txt").await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }

    #[tokio::test]
    async fn read_nonexistent_returns_not_found() {
        let (_dir, storage) = setup().await;
        let err = storage.read("nonexistent.txt").await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }

    #[tokio::test]
    async fn exists_returns_false_for_missing() {
        let (_dir, storage) = setup().await;
        assert!(!storage.exists("nope.txt").await.unwrap());
    }

    #[tokio::test]
    async fn file_metadata_returns_correct_size() {
        let (_dir, storage) = setup().await;
        let data = b"metadata test content";

        storage.store("meta.txt", data).await.unwrap();
        let meta = storage.file_metadata("meta.txt").await.unwrap();

        assert_eq!(meta.size, data.len() as u64);
        // modified should be recent (within last minute)
        let age = Utc::now() - meta.modified;
        assert!(age.num_seconds() < 60);
    }

    #[tokio::test]
    async fn file_metadata_nonexistent_returns_not_found() {
        let (_dir, storage) = setup().await;
        let err = storage.file_metadata("missing.txt").await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }

    #[tokio::test]
    async fn rejects_path_traversal() {
        let (_dir, storage) = setup().await;
        let result = storage.store("../../etc/passwd", b"evil").await;
        assert!(matches!(result, Err(StorageError::PathConflict(_))));
    }

    #[tokio::test]
    async fn rejects_absolute_path() {
        let (_dir, storage) = setup().await;
        let result = storage.store("/etc/passwd", b"evil").await;
        assert!(matches!(result, Err(StorageError::PathConflict(_))));
    }

    #[tokio::test]
    async fn rejects_null_bytes_in_path() {
        let (_dir, storage) = setup().await;
        let result = storage.store("file\0.txt", b"evil").await;
        assert!(matches!(result, Err(StorageError::PathConflict(_))));
    }

    #[tokio::test]
    async fn store_empty_file() {
        let (_dir, storage) = setup().await;
        let stored = storage.store("empty.bin", b"").await.unwrap();
        assert_eq!(stored.size, 0);

        let data = storage.read("empty.bin").await.unwrap();
        assert!(data.is_empty());
    }

    #[tokio::test]
    async fn store_large_data() {
        let (_dir, storage) = setup().await;
        let data = vec![0xABu8; 1024 * 1024]; // 1 MiB

        let stored = storage.store("large.bin", &data).await.unwrap();
        assert_eq!(stored.size, 1024 * 1024);

        let read_data = storage.read("large.bin").await.unwrap();
        assert_eq!(read_data, data);
    }

    #[test]
    fn sha256_hex_known_value() {
        // SHA-256 of empty string
        let hash = sha256_hex(b"");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn normalize_path_resolves_parent_dirs() {
        let path = Path::new("/a/b/../c/./d");
        assert_eq!(normalize_path(path), PathBuf::from("/a/c/d"));
    }

    #[test]
    fn sibling_temp_path_format() {
        let path = Path::new("/storage/A/Author/book.epub");
        let temp = sibling_temp_path(path);
        assert_eq!(temp, PathBuf::from("/storage/A/Author/.book.epub.tmp"));
    }
}
