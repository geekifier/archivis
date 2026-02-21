use std::io;

/// Top-level error type aggregating all domain-area errors.
#[derive(Debug, thiserror::Error)]
pub enum ArchivisError {
    #[error(transparent)]
    Db(#[from] DbError),

    #[error(transparent)]
    Format(#[from] FormatError),

    #[error(transparent)]
    Storage(#[from] StorageError),

    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error(transparent)]
    Task(#[from] TaskError),

    #[error(transparent)]
    Metadata(#[from] MetadataError),
}

/// Database layer errors.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database connection failed: {0}")]
    Connection(String),

    #[error("query failed: {0}")]
    Query(String),

    #[error("migration failed: {0}")]
    Migration(String),

    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },

    #[error("constraint violation: {0}")]
    Constraint(String),

    #[error("transaction failed: {0}")]
    Transaction(String),
}

/// Ebook format detection and parsing errors.
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("unsupported format")]
    Unsupported,

    #[error("failed to detect format: {0}")]
    Detection(String),

    #[error("failed to parse {format}: {message}")]
    Parse { format: String, message: String },

    #[error("metadata extraction failed: {0}")]
    MetadataExtraction(String),

    #[error(transparent)]
    Io(#[from] io::Error),
}

/// File storage errors.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("file not found: {0}")]
    NotFound(String),

    #[error("storage path conflict: {0}")]
    PathConflict(String),

    #[error("hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },

    #[error("storage I/O error: {source}")]
    Io {
        #[from]
        source: io::Error,
    },

    #[error("insufficient storage space")]
    InsufficientSpace,
}

/// Authentication and authorization errors.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,

    #[error("session expired")]
    SessionExpired,

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden: insufficient permissions")]
    Forbidden,

    #[error("user already exists: {0}")]
    UserExists(String),

    #[error("password does not meet requirements: {0}")]
    WeakPassword(String),

    #[error("auth error: {0}")]
    Internal(String),
}

/// Background task system errors.
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("task not found: {0}")]
    NotFound(String),

    #[error("task failed: {0}")]
    Failed(String),

    #[error("task cancelled")]
    Cancelled,

    #[error("task queue full")]
    QueueFull,

    #[error("task error: {0}")]
    Internal(String),
}

/// Metadata resolution and provider errors.
#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("no metadata found")]
    NotFound,

    #[error("provider {provider} failed: {message}")]
    ProviderError { provider: String, message: String },

    #[error("identifier validation failed: {0}")]
    InvalidIdentifier(String),

    #[error("metadata conflict: {0}")]
    Conflict(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_error_display() {
        let err = DbError::NotFound {
            entity: "book",
            id: "abc-123".into(),
        };
        assert_eq!(err.to_string(), "book not found: abc-123");
    }

    #[test]
    fn db_error_into_archivis() {
        let db_err = DbError::Connection("timeout".into());
        let err: ArchivisError = db_err.into();
        assert!(matches!(err, ArchivisError::Db(DbError::Connection(_))));
        assert_eq!(err.to_string(), "database connection failed: timeout");
    }

    #[test]
    fn format_error_display() {
        let err = FormatError::Parse {
            format: "EPUB".into(),
            message: "invalid OPF".into(),
        };
        assert_eq!(err.to_string(), "failed to parse EPUB: invalid OPF");
    }

    #[test]
    fn storage_error_hash_mismatch() {
        let err = StorageError::HashMismatch {
            expected: "abc".into(),
            actual: "def".into(),
        };
        assert_eq!(err.to_string(), "hash mismatch: expected abc, got def");
    }

    #[test]
    fn auth_error_into_archivis() {
        let auth_err = AuthError::InvalidCredentials;
        let err: ArchivisError = auth_err.into();
        assert!(matches!(
            err,
            ArchivisError::Auth(AuthError::InvalidCredentials)
        ));
    }

    #[test]
    fn task_error_display() {
        let err = TaskError::Failed("import aborted".into());
        assert_eq!(err.to_string(), "task failed: import aborted");
    }

    #[test]
    fn metadata_error_provider() {
        let err = MetadataError::ProviderError {
            provider: "Hardcover".into(),
            message: "rate limited".into(),
        };
        assert_eq!(err.to_string(), "provider Hardcover failed: rate limited");
    }

    #[test]
    fn error_chain_from_conversions() {
        // Verify transitive conversion: FormatError → ArchivisError
        let fmt_err = FormatError::Unsupported;
        let archivis_err: ArchivisError = fmt_err.into();
        assert!(matches!(
            archivis_err,
            ArchivisError::Format(FormatError::Unsupported)
        ));

        let storage_err = StorageError::NotFound("missing.epub".into());
        let archivis_err: ArchivisError = storage_err.into();
        assert!(matches!(
            archivis_err,
            ArchivisError::Storage(StorageError::NotFound(_))
        ));
    }

    #[test]
    fn io_error_converts_to_format_error() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file missing");
        let fmt_err: FormatError = io_err.into();
        assert!(matches!(fmt_err, FormatError::Io(_)));
    }

    #[test]
    fn io_error_converts_to_storage_error() {
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
        let storage_err: StorageError = io_err.into();
        assert!(matches!(storage_err, StorageError::Io { .. }));
    }
}
