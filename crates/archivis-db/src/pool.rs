use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use crate::repositories::BookRepository;
use archivis_core::errors::DbError;
use sqlx::migrate::{Migration, Migrator};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

/// Type alias for the `SQLite` connection pool used throughout the application.
pub type DbPool = SqlitePool;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

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
    reconcile_legacy_migration_history(pool).await?;

    MIGRATOR
        .run(pool)
        .await
        .map_err(|e| DbError::Migration(e.to_string()))?;

    // Backfill norm_title for existing rows after migration adds the column.
    BookRepository::backfill_norm_titles(pool).await?;

    tracing::info!("database migrations applied");
    Ok(())
}

async fn reconcile_legacy_migration_history(pool: &DbPool) -> Result<(), DbError> {
    if !sqlite_object_exists(pool, "table", "_sqlx_migrations").await?
        || !sqlite_object_exists(pool, "table", "books").await?
    {
        return Ok(());
    }

    let applied_versions: HashSet<i64> = sqlx::query_scalar("SELECT version FROM _sqlx_migrations")
        .fetch_all(pool)
        .await
        .map_err(|e| DbError::Query(format!("failed to read applied migrations: {e}")))?
        .into_iter()
        .collect();

    let book_columns = table_columns(pool, "books").await?;

    if !applied_versions.contains(&12)
        && book_columns.contains("norm_title")
        && sqlite_object_exists(pool, "index", "idx_books_norm_prefix").await?
    {
        mark_migration_applied(pool, 12).await?;
    }

    if !applied_versions.contains(&13)
        && book_columns.contains("subtitle")
        && subtitle_fts_triggers_present(pool).await?
    {
        mark_migration_applied(pool, 13).await?;
    }

    Ok(())
}

async fn sqlite_object_exists(
    pool: &DbPool,
    object_type: &str,
    name: &str,
) -> Result<bool, DbError> {
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(
            SELECT 1
            FROM sqlite_master
            WHERE type = ? AND name = ?
        )",
    )
    .bind(object_type)
    .bind(name)
    .fetch_one(pool)
    .await
    .map_err(|e| DbError::Query(format!("failed to inspect sqlite_master for {name}: {e}")))?;

    Ok(exists == 1)
}

async fn table_columns(pool: &DbPool, table: &str) -> Result<HashSet<String>, DbError> {
    sqlx::query_scalar::<_, String>("SELECT name FROM pragma_table_info(?)")
        .bind(table)
        .fetch_all(pool)
        .await
        .map(|columns| columns.into_iter().collect())
        .map_err(|e| DbError::Query(format!("failed to inspect table columns for {table}: {e}")))
}

async fn subtitle_fts_triggers_present(pool: &DbPool) -> Result<bool, DbError> {
    for trigger in [
        "books_fts_insert",
        "books_fts_update",
        "book_authors_fts_insert",
        "book_authors_fts_delete",
        "authors_fts_update",
    ] {
        let sql = sqlx::query_scalar::<_, String>(
            "SELECT sql
             FROM sqlite_master
             WHERE type = 'trigger' AND name = ?",
        )
        .bind(trigger)
        .fetch_optional(pool)
        .await
        .map_err(|e| DbError::Query(format!("failed to inspect trigger {trigger}: {e}")))?;

        if !sql.is_some_and(|definition| definition.contains("subtitle")) {
            return Ok(false);
        }
    }

    Ok(true)
}

async fn mark_migration_applied(pool: &DbPool, version: i64) -> Result<(), DbError> {
    let migration = MIGRATOR
        .iter()
        .find(|migration| migration.version == version)
        .ok_or_else(|| DbError::Migration(format!("missing embedded migration {version}")))?;

    insert_migration_record(pool, migration).await?;

    tracing::warn!(
        version,
        description = migration.description.as_ref(),
        "reconciled legacy migration history for pre-applied schema drift"
    );

    Ok(())
}

async fn insert_migration_record(pool: &DbPool, migration: &Migration) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO _sqlx_migrations (
            version, description, success, checksum, execution_time
         ) VALUES (?, ?, 1, ?, 0)",
    )
    .bind(migration.version)
    .bind(migration.description.as_ref())
    .bind(migration.checksum.as_ref())
    .execute(pool)
    .await
    .map_err(|e| {
        DbError::Migration(format!(
            "failed to record migration {} as applied: {e}",
            migration.version
        ))
    })?;

    Ok(())
}

/// Lightweight connectivity check: acquires a connection and runs `SELECT 1`.
pub async fn ping(pool: &DbPool) -> Result<(), DbError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
        .map_err(|e| DbError::Query(format!("ping failed: {e}")))?;
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

    async fn create_sqlx_migrations_table(pool: &DbPool) {
        sqlx::raw_sql(
            r"
            CREATE TABLE _sqlx_migrations (
                version BIGINT PRIMARY KEY,
                description TEXT NOT NULL,
                installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                success BOOLEAN NOT NULL,
                checksum BLOB NOT NULL,
                execution_time BIGINT NOT NULL
            );
            ",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn apply_migrations_through(pool: &DbPool, version: i64) {
        create_sqlx_migrations_table(pool).await;

        for migration in MIGRATOR
            .iter()
            .filter(|migration| migration.version <= version)
        {
            sqlx::raw_sql(migration.sql.as_ref())
                .execute(pool)
                .await
                .unwrap();
            insert_migration_record(pool, migration).await.unwrap();
        }
    }

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
            "bookmarks",
            "books",
            "books_fts",
            "duplicate_links",
            "identification_candidates",
            "identifiers",
            "publishers",
            "reading_progress",
            "resolution_runs",
            "series",
            "sessions",
            "settings",
            "tags",
            "tasks",
            "users",
            "watched_directories",
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

    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn resolution_foundation_migration_backfills_books_and_accepts_superseded() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("pre_task1.db");
        let pool = create_pool(&db_path).await.unwrap();

        sqlx::raw_sql(
            r"
            CREATE TABLE books (
                id TEXT PRIMARY KEY NOT NULL,
                title TEXT NOT NULL,
                sort_title TEXT NOT NULL,
                description TEXT,
                language TEXT,
                publication_date TEXT,
                publisher_id TEXT,
                added_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                rating REAL,
                page_count INTEGER,
                metadata_status TEXT NOT NULL DEFAULT 'unidentified'
                    CHECK (metadata_status IN ('identified', 'needs_review', 'unidentified')),
                metadata_confidence REAL NOT NULL DEFAULT 0.0
                    CHECK (metadata_confidence >= 0.0 AND metadata_confidence <= 1.0),
                cover_path TEXT,
                subtitle TEXT,
                norm_title TEXT NOT NULL DEFAULT ''
            );

            CREATE TRIGGER books_updated_at AFTER UPDATE ON books
            FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at BEGIN
                UPDATE books SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                WHERE id = NEW.id;
            END;

            CREATE TABLE identification_candidates (
                id TEXT PRIMARY KEY NOT NULL,
                book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
                provider_name TEXT NOT NULL,
                score REAL NOT NULL DEFAULT 0.0,
                metadata TEXT NOT NULL,
                match_reasons TEXT,
                status TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'applied', 'rejected')),
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );

            INSERT INTO books (
                id, title, subtitle, sort_title, description, added_at, updated_at,
                metadata_status, metadata_confidence, norm_title
            ) VALUES (
                'book-1', 'Dune', NULL, 'Dune', NULL,
                '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z',
                'identified', 1.0, 'dune'
            );

            INSERT INTO identification_candidates (
                id, book_id, provider_name, score, metadata, match_reasons, status, created_at
            ) VALUES (
                'candidate-1', 'book-1', 'open_library', 0.95, '{}', '[]', 'pending',
                '2024-01-01T00:00:00Z'
            );
            ",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(include_str!("../migrations/014_resolution_foundations.sql"))
            .execute(&pool)
            .await
            .unwrap();

        let book = sqlx::query!(
            "SELECT resolution_state, resolution_outcome, resolution_requested_at, resolution_requested_reason, metadata_locked, metadata_provenance FROM books WHERE id = 'book-1'"
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(book.resolution_state, "pending");
        assert!(book.resolution_outcome.is_none());
        assert_eq!(book.resolution_requested_at, "2024-01-01T00:00:00Z");
        assert_eq!(
            book.resolution_requested_reason.as_deref(),
            Some("migration_backfill")
        );
        assert_eq!(book.metadata_locked, 0);
        assert_eq!(book.metadata_provenance, "{}");

        sqlx::query!(
            "INSERT INTO identification_candidates (id, book_id, provider_name, score, metadata, status, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            "candidate-2",
            "book-1",
            "open_library",
            0.1_f32,
            "{}",
            "superseded",
            "2024-01-02T00:00:00Z",
        )
        .execute(&pool)
        .await
        .unwrap();

        let candidate = sqlx::query!(
            "SELECT run_id, disputes, status FROM identification_candidates WHERE id = 'candidate-1'"
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(candidate.run_id.is_none());
        assert!(candidate.disputes.is_none());
        assert_eq!(candidate.status, "pending");

        pool.close().await;
    }

    #[tokio::test]
    async fn ingest_quality_score_migration_renames_column_and_preserves_value() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("pre_task2.db");
        let pool = create_pool(&db_path).await.unwrap();

        sqlx::raw_sql(
            r"
            CREATE TABLE books (
                id TEXT PRIMARY KEY NOT NULL,
                title TEXT NOT NULL,
                sort_title TEXT NOT NULL,
                description TEXT,
                language TEXT,
                publication_date TEXT,
                publisher_id TEXT,
                added_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                rating REAL,
                page_count INTEGER,
                metadata_status TEXT NOT NULL DEFAULT 'unidentified'
                    CHECK (metadata_status IN ('identified', 'needs_review', 'unidentified')),
                metadata_confidence REAL NOT NULL DEFAULT 0.0
                    CHECK (metadata_confidence >= 0.0 AND metadata_confidence <= 1.0),
                cover_path TEXT,
                subtitle TEXT,
                norm_title TEXT NOT NULL DEFAULT '',
                resolution_state TEXT NOT NULL DEFAULT 'pending'
                    CHECK (resolution_state IN ('pending', 'running', 'done', 'failed')),
                resolution_outcome TEXT
                    CHECK (resolution_outcome IN ('confirmed', 'enriched', 'disputed', 'ambiguous', 'unmatched')),
                resolution_requested_at TEXT NOT NULL DEFAULT '',
                resolution_requested_reason TEXT,
                last_resolved_at TEXT,
                last_resolution_run_id TEXT,
                metadata_locked INTEGER NOT NULL DEFAULT 0,
                metadata_provenance TEXT NOT NULL DEFAULT '{}'
            );

            INSERT INTO books (
                id, title, sort_title, added_at, updated_at, metadata_status,
                metadata_confidence, norm_title, resolution_requested_at
            ) VALUES (
                'book-1', 'Dune', 'Dune', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z',
                'identified', 0.73, 'dune', '2024-01-01T00:00:00Z'
            );
            ",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(include_str!("../migrations/015_ingest_quality_score.sql"))
            .execute(&pool)
            .await
            .unwrap();

        let score: f64 =
            sqlx::query_scalar("SELECT ingest_quality_score FROM books WHERE id = 'book-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!((score - 0.73).abs() < f64::EPSILON);

        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('books')")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(columns.iter().any(|name| name == "ingest_quality_score"));
        assert!(!columns.iter().any(|name| name == "metadata_confidence"));

        pool.close().await;
    }

    #[tokio::test]
    async fn resolution_cleanup_migration_rewrites_legacy_task_types() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("pre_task13.db");
        let pool = create_pool(&db_path).await.unwrap();

        sqlx::raw_sql(
            r"
            CREATE TABLE tasks (
                id TEXT PRIMARY KEY NOT NULL,
                task_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                status TEXT NOT NULL,
                progress INTEGER NOT NULL DEFAULT 0,
                message TEXT,
                result TEXT,
                created_at TEXT NOT NULL,
                started_at TEXT,
                completed_at TEXT,
                error_message TEXT,
                parent_task_id TEXT
            );

            INSERT INTO tasks (
                id, task_type, payload, status, progress, created_at
            ) VALUES (
                'task-1', 'identify_book', '{}', 'completed', 100, '2024-01-01T00:00:00Z'
            );
            ",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(include_str!("../migrations/016_resolution_cleanup.sql"))
            .execute(&pool)
            .await
            .unwrap();

        let task_type: String =
            sqlx::query_scalar("SELECT task_type FROM tasks WHERE id = 'task-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(task_type, "resolve_book");

        pool.close().await;
    }

    #[tokio::test]
    async fn run_migrations_reconciles_preapplied_norm_title_and_subtitle_migrations() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("schema_drift.db");
        let pool = create_pool(&db_path).await.unwrap();

        apply_migrations_through(&pool, 11).await;

        sqlx::raw_sql(include_str!("../migrations/012_norm_title.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../migrations/013_subtitle.sql"))
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO tasks (
                id, task_type, payload, status, progress, created_at
             ) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind("task-1")
        .bind("identify_book")
        .bind("{}")
        .bind("completed")
        .bind(100_i64)
        .bind("2024-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .unwrap();

        run_migrations(&pool).await.unwrap();

        let applied_versions: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(applied_versions, (1..=18).collect::<Vec<_>>());

        let columns = table_columns(&pool, "books").await.unwrap();
        assert!(columns.contains("ingest_quality_score"));
        assert!(!columns.contains("metadata_confidence"));
        assert!(columns.contains("resolution_state"));
        assert!(columns.contains("resolution_requested_at"));

        let task_type: String =
            sqlx::query_scalar("SELECT task_type FROM tasks WHERE id = 'task-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(task_type, "resolve_book");

        pool.close().await;
    }
}
