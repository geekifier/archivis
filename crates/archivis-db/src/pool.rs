use std::path::Path;
use std::time::Duration;

use archivis_core::errors::DbError;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

/// Type alias for the `SQLite` connection pool used throughout the application.
pub type DbPool = SqlitePool;

/// Create a configured `SQLite` connection pool.
///
/// Sets the following PRAGMAs on every connection:
/// - `journal_mode=WAL` — write-ahead logging for concurrent reads
/// - `foreign_keys=ON` — enforce referential integrity
/// - `busy_timeout=5000` — wait up to 5s for locks instead of failing immediately
/// - `synchronous=NORMAL` — safe with WAL, better performance than FULL
///
/// Creates the database file and parent directories if they do not exist.
pub async fn create_pool(db_path: &Path) -> Result<DbPool, DbError> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            DbError::Connection(format!(
                "failed to create database directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    let connect_options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(5000));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .min_connections(1)
        .idle_timeout(Duration::from_secs(300))
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(connect_options)
        .await
        .map_err(|e| DbError::Connection(e.to_string()))?;

    verify_pragmas(&pool).await?;

    tracing::info!(path = %db_path.display(), "database pool initialized");

    Ok(pool)
}

/// Run embedded database migrations.
pub async fn run_migrations(pool: &DbPool) -> Result<(), DbError> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| DbError::Migration(e.to_string()))?;

    tracing::info!("database migrations applied");
    Ok(())
}

/// Verify that PRAGMAs are correctly applied.
async fn verify_pragmas(pool: &DbPool) -> Result<(), DbError> {
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(format!("failed to check journal_mode: {e}")))?;

    if journal_mode.to_lowercase() != "wal" {
        return Err(DbError::Connection(format!(
            "expected journal_mode=wal, got {journal_mode}"
        )));
    }

    let foreign_keys: i32 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(format!("failed to check foreign_keys: {e}")))?;

    if foreign_keys != 1 {
        return Err(DbError::Connection(
            "foreign_keys PRAGMA is not enabled".into(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn pool_connects_and_pragmas_set() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");

        let pool = create_pool(&db_path).await.unwrap();

        // Verify WAL mode
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(journal_mode.to_lowercase(), "wal");

        // Verify foreign keys
        let fk: i32 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(fk, 1);

        // Verify busy_timeout (5000ms)
        let timeout: i32 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(timeout, 5000);

        // Verify synchronous=NORMAL (1)
        let sync: i32 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(sync, 1);

        pool.close().await;
    }

    #[tokio::test]
    async fn pool_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("nested").join("deep").join("test.db");

        let pool = create_pool(&db_path).await.unwrap();
        assert!(db_path.exists());

        pool.close().await;
    }

    #[tokio::test]
    async fn migrations_apply_successfully() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let pool = create_pool(&db_path).await.unwrap();

        run_migrations(&pool).await.unwrap();

        // Verify all expected tables exist (exclude FTS internal tables and sqlx migration tables)
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_sqlx_%' AND name NOT LIKE 'books_fts_%' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        let expected = [
            "authors",
            "book_authors",
            "book_files",
            "book_series",
            "book_tags",
            "books",
            "books_fts",
            "identifiers",
            "publishers",
            "series",
            "sessions",
            "tags",
            "tasks",
            "users",
        ];

        assert_eq!(tables, expected);

        pool.close().await;
    }

    #[tokio::test]
    async fn fts_indexes_book_on_insert() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let pool = create_pool(&db_path).await.unwrap();
        run_migrations(&pool).await.unwrap();

        // Insert a book
        sqlx::query("INSERT INTO books (id, title, sort_title, description) VALUES ('b1', 'Dune', 'Dune', 'A desert planet saga')")
            .execute(&pool)
            .await
            .unwrap();

        // Insert an author and link
        sqlx::query("INSERT INTO authors (id, name, sort_name) VALUES ('a1', 'Frank Herbert', 'Herbert, Frank')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO book_authors (book_id, author_id) VALUES ('b1', 'a1')")
            .execute(&pool)
            .await
            .unwrap();

        // Search FTS
        let results: Vec<String> =
            sqlx::query_scalar("SELECT title FROM books_fts WHERE books_fts MATCH 'dune'")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(results, vec!["Dune"]);

        // Search by author
        let results: Vec<String> =
            sqlx::query_scalar("SELECT title FROM books_fts WHERE books_fts MATCH 'herbert'")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(results, vec!["Dune"]);

        pool.close().await;
    }

    #[tokio::test]
    async fn foreign_keys_enforced() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let pool = create_pool(&db_path).await.unwrap();
        run_migrations(&pool).await.unwrap();

        // Try to insert a book_file referencing a non-existent book
        let result = sqlx::query(
            "INSERT INTO book_files (id, book_id, format, storage_path, file_size, hash) VALUES ('f1', 'nonexistent', 'epub', 'path', 100, 'hash')"
        )
        .execute(&pool)
        .await;

        assert!(
            result.is_err(),
            "foreign key constraint should prevent insert"
        );

        pool.close().await;
    }

    #[tokio::test]
    async fn pool_handles_existing_db() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");

        // Create and close
        let pool = create_pool(&db_path).await.unwrap();
        pool.close().await;

        // Reopen — should work without error
        let pool2 = create_pool(&db_path).await.unwrap();
        pool2.close().await;
    }
}
