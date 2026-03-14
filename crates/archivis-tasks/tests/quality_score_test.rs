use archivis_core::models::{Author, Book, Identifier, IdentifierType, MetadataSource, Series};
use archivis_core::scoring::{
    compute_quality_score, is_valid_identifier_by_type, BALANCED_WEIGHTS,
};
use archivis_db::{AuthorRepository, BookRepository, IdentifierRepository, SeriesRepository};
use archivis_tasks::resolve::{compute_and_persist_quality_score, refresh_metadata_quality_score};
use tempfile::TempDir;

async fn setup_pool(tmp: &TempDir) -> archivis_db::DbPool {
    let db_path = tmp.path().join("test.db");
    let pool = archivis_db::create_pool(&db_path).await.unwrap();
    archivis_db::run_migrations(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn refresh_on_isbn_addition_increases_score() {
    let tmp = TempDir::new().unwrap();
    let pool = setup_pool(&tmp).await;

    // Create a book with title + author (no identifiers)
    let book = Book::new("Dune");
    BookRepository::create(&pool, &book).await.unwrap();
    let author = Author::new("Frank Herbert");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    // Compute initial score (title + author, no ISBN)
    let score_before = refresh_metadata_quality_score(&pool, book.id)
        .await
        .unwrap();

    // Add a valid ISBN-13 identifier (simulating ISBN scan discovery)
    let isbn = Identifier::new(
        book.id,
        IdentifierType::Isbn13,
        "9783161484100",
        MetadataSource::ContentScan,
        0.5,
    );
    IdentifierRepository::create(&pool, &isbn).await.unwrap();

    // Refresh after ISBN addition
    let score_after = refresh_metadata_quality_score(&pool, book.id)
        .await
        .unwrap();

    assert!(
        score_after > score_before,
        "score should increase after adding ISBN: before={score_before}, after={score_after}"
    );

    // Verify the ISBN bonus is reflected (0.4 + 0.3 = 0.7 base)
    assert!(
        (score_after - score_before - 0.4).abs() < f32::EPSILON,
        "ISBN should add 0.4 bonus: delta={}",
        score_after - score_before
    );
}

#[tokio::test]
async fn refresh_on_isbn_removal_decreases_score() {
    let tmp = TempDir::new().unwrap();
    let pool = setup_pool(&tmp).await;

    // Create book with title + author + ISBN
    let book = Book::new("Dune");
    BookRepository::create(&pool, &book).await.unwrap();
    let author = Author::new("Frank Herbert");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();
    let isbn = Identifier::new(
        book.id,
        IdentifierType::Isbn13,
        "9783161484100",
        MetadataSource::Embedded,
        0.9,
    );
    IdentifierRepository::create(&pool, &isbn).await.unwrap();

    let score_with_isbn = refresh_metadata_quality_score(&pool, book.id)
        .await
        .unwrap();

    // Remove the ISBN (simulating undo)
    IdentifierRepository::delete(&pool, isbn.id).await.unwrap();

    let score_without_isbn = refresh_metadata_quality_score(&pool, book.id)
        .await
        .unwrap();

    assert!(
        score_with_isbn > score_without_isbn,
        "score should decrease after removing ISBN"
    );
}

#[tokio::test]
async fn refresh_includes_cover_richness() {
    let tmp = TempDir::new().unwrap();
    let pool = setup_pool(&tmp).await;

    // Create book without cover
    let mut book = Book::new("Test Book");
    BookRepository::create(&pool, &book).await.unwrap();
    let author = Author::new("Test Author");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let score_no_cover = refresh_metadata_quality_score(&pool, book.id)
        .await
        .unwrap();

    // Add cover path (simulating cover transfer from merge)
    book.cover_path = Some("covers/test.jpg".into());
    BookRepository::update(&pool, &book).await.unwrap();

    let score_with_cover = refresh_metadata_quality_score(&pool, book.id)
        .await
        .unwrap();

    assert!(
        score_with_cover > score_no_cover,
        "score should increase with cover: no_cover={score_no_cover}, with_cover={score_with_cover}"
    );
}

#[tokio::test]
async fn refresh_includes_series_richness() {
    let tmp = TempDir::new().unwrap();
    let pool = setup_pool(&tmp).await;

    let book = Book::new("The Fellowship of the Ring");
    BookRepository::create(&pool, &book).await.unwrap();
    let author = Author::new("J.R.R. Tolkien");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let score_no_series = refresh_metadata_quality_score(&pool, book.id)
        .await
        .unwrap();

    // Add series
    let series = Series {
        id: uuid::Uuid::new_v4(),
        name: "The Lord of the Rings".into(),
        description: None,
    };
    SeriesRepository::create(&pool, &series).await.unwrap();
    BookRepository::add_series(&pool, book.id, series.id, Some(1.0))
        .await
        .unwrap();

    let score_with_series = refresh_metadata_quality_score(&pool, book.id)
        .await
        .unwrap();

    assert!(
        score_with_series > score_no_series,
        "score should increase with series"
    );
}

#[tokio::test]
async fn score_persisted_in_database() {
    let tmp = TempDir::new().unwrap();
    let pool = setup_pool(&tmp).await;

    let book = Book::new("Persisted Score Book");
    BookRepository::create(&pool, &book).await.unwrap();

    // Initially NULL
    let loaded = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert!(loaded.metadata_quality_score.is_none());

    // After refresh, persisted
    let score = refresh_metadata_quality_score(&pool, book.id)
        .await
        .unwrap();

    let loaded = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert!(loaded.metadata_quality_score.is_some());
    assert!(
        (loaded.metadata_quality_score.unwrap() - score).abs() < f32::EPSILON,
        "persisted score should match computed score"
    );
}

#[tokio::test]
async fn backfill_populates_null_scores() {
    let tmp = TempDir::new().unwrap();
    let pool = setup_pool(&tmp).await;

    // Create several books with NULL score
    for title in &["Book A", "Book B", "Book C"] {
        let book = Book::new(*title);
        BookRepository::create(&pool, &book).await.unwrap();
    }

    // All should be NULL
    let null_ids = BookRepository::list_ids_without_quality_score(&pool, 100)
        .await
        .unwrap();
    assert_eq!(null_ids.len(), 3);

    // Run backfill
    let count = archivis_tasks::resolve::backfill_metadata_quality_scores(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3);

    // All should now have scores
    let null_ids = BookRepository::list_ids_without_quality_score(&pool, 100)
        .await
        .unwrap();
    assert!(null_ids.is_empty());
}

// ── Unit tests for shared scoring engine parity ─────────────────

#[test]
fn is_valid_identifier_parity_isbn13() {
    // Valid ISBN-13
    assert!(is_valid_identifier_by_type(
        IdentifierType::Isbn13,
        "9783161484100"
    ));
    // Invalid checksum
    assert!(!is_valid_identifier_by_type(
        IdentifierType::Isbn13,
        "9783161484109"
    ));
    // Placeholder
    assert!(!is_valid_identifier_by_type(
        IdentifierType::Isbn13,
        "0000000000000"
    ));
}

#[test]
fn is_valid_identifier_parity_isbn10() {
    assert!(is_valid_identifier_by_type(
        IdentifierType::Isbn10,
        "0306406152"
    ));
    assert!(is_valid_identifier_by_type(
        IdentifierType::Isbn10,
        "080442957X"
    ));
    assert!(!is_valid_identifier_by_type(
        IdentifierType::Isbn10,
        "0306406153"
    ));
}

#[test]
fn is_valid_identifier_parity_asin() {
    // ASIN is always valid if present
    assert!(is_valid_identifier_by_type(
        IdentifierType::Asin,
        "B000FA64PK"
    ));
}

#[test]
fn live_score_uses_balanced_weights_full() {
    use archivis_core::scoring::QualitySignals;

    let signals = QualitySignals {
        has_title: true,
        has_author: true,
        has_strong_identifier: true,
        richness_present: 7,
        richness_total: 7,
        context_bonus: 0.0,
    };
    let score = compute_quality_score(&signals, &BALANCED_WEIGHTS);
    // 0.4 (isbn) + 0.3 (title+author) + 0.30 (7/7 richness) = 1.0
    assert!(
        (score - 1.0).abs() < f32::EPSILON,
        "full book should score 1.0, got {score}"
    );
}

#[tokio::test]
async fn compute_and_persist_from_preloaded_bwr() {
    let tmp = TempDir::new().unwrap();
    let pool = setup_pool(&tmp).await;

    // Create book with title + author
    let book = Book::new("Preloaded Test");
    BookRepository::create(&pool, &book).await.unwrap();
    let author = Author::new("Test Author");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    // Load BWR, compute via preloaded path
    let bwr = BookRepository::get_with_relations(&pool, book.id)
        .await
        .unwrap();
    let score = compute_and_persist_quality_score(&pool, &bwr)
        .await
        .unwrap();

    // Score should reflect title + author (0.3)
    assert!(
        score > 0.0,
        "score should be > 0 for book with title + author"
    );

    // Verify persisted in DB
    let loaded = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert!(loaded.metadata_quality_score.is_some());
    assert!(
        (loaded.metadata_quality_score.unwrap() - score).abs() < f32::EPSILON,
        "persisted score should match returned score"
    );

    // BWR is not mutated (passed by shared reference)
    assert!(
        bwr.book.metadata_quality_score.is_none(),
        "original BWR should not be mutated"
    );
}

#[test]
fn live_score_uses_balanced_weights_minimal() {
    use archivis_core::scoring::QualitySignals;

    let signals = QualitySignals {
        has_title: true,
        has_author: false,
        has_strong_identifier: false,
        richness_present: 0,
        richness_total: 7,
        context_bonus: 0.0,
    };
    let score = compute_quality_score(&signals, &BALANCED_WEIGHTS);
    // 0.1 (title only, no author)
    assert!(
        (score - 0.1).abs() < f32::EPSILON,
        "title-only book should score 0.1, got {score}"
    );
}
