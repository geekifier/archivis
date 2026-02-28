use archivis_core::models::{Book, BookFile, BookFormat};
use archivis_db::{BookFileRepository, BookRepository};
use archivis_storage::local::LocalStorage;
use archivis_tasks::merge::{MergeOptions, MergeService};
use tempfile::TempDir;

/// Set up a test environment with DB, storage, and merge service.
async fn setup_merge_env(tmp: &TempDir) -> (MergeService<LocalStorage>, archivis_db::DbPool) {
    let db_path = tmp.path().join("test.db");
    let pool = archivis_db::create_pool(&db_path).await.unwrap();
    archivis_db::run_migrations(&pool).await.unwrap();

    let storage_dir = tmp.path().join("storage");
    let storage = LocalStorage::new(&storage_dir).await.unwrap();

    let service = MergeService::new(pool.clone(), storage, tmp.path().join("data"));
    (service, pool)
}

#[tokio::test]
async fn merge_deduplicates_same_hash_files() {
    let tmp = TempDir::new().unwrap();
    let (service, pool) = setup_merge_env(&tmp).await;

    // Drop the unique hash index so we can simulate the scenario where
    // two book_file rows share the same hash (e.g., from a prior bug).
    sqlx::query("DROP INDEX IF EXISTS idx_book_files_hash")
        .execute(&pool)
        .await
        .unwrap();

    // Create two books, each with a file that has the same hash
    let book_a = Book::new("Test Book A");
    BookRepository::create(&pool, &book_a).await.unwrap();

    let book_b = Book::new("Test Book B");
    BookRepository::create(&pool, &book_b).await.unwrap();

    let shared_hash = "deadbeef".repeat(8);

    let file_a = BookFile::new(
        book_a.id,
        BookFormat::Epub,
        "books/a/test.epub",
        1000,
        &shared_hash,
        None,
    );
    BookFileRepository::create(&pool, &file_a).await.unwrap();

    // Small delay to ensure different added_at timestamps
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let file_b = BookFile::new(
        book_b.id,
        BookFormat::Epub,
        "books/b/test.epub",
        1000,
        &shared_hash,
        None,
    );
    BookFileRepository::create(&pool, &file_b).await.unwrap();

    // Merge B into A
    let _result = service
        .merge_books(book_a.id, book_b.id, MergeOptions::default())
        .await
        .unwrap();

    // After merge, primary should have exactly 1 file (deduplicated)
    let files = BookFileRepository::get_by_book_id(&pool, book_a.id)
        .await
        .unwrap();
    assert_eq!(
        files.len(),
        1,
        "duplicate files with same hash should be deduplicated after merge"
    );
    assert_eq!(files[0].hash, shared_hash);

    pool.close().await;
}
