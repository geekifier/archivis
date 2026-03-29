use archivis_core::models::{
    Author, Book, BookFile, BookFormat, Bookmark, IdentificationCandidate, Identifier,
    IdentifierType, LibraryFilterState, MetadataSource, MetadataStatus, Publisher,
    ResolutionOutcome, ResolutionRun, ResolutionState, Series, Tag, TagMatchMode, User, UserRole,
    WatchMode,
};
use archivis_core::search_query::parse_search_query;
use archivis_db::{
    create_pool, run_migrations, AuthorRepository, BookFileRepository, BookFilter, BookRepository,
    BookmarkRepository, CandidateRepository, DbPool, IdentifierRepository, PaginationParams,
    PublisherRepository, QueryWarning, ReadingProgressRepository, ResolutionRunRepository,
    SearchResolver, SeriesRepository, SortOrder, StatsRepository, TagRepository, UserRepository,
    WatchedDirectoryRepository,
};
use chrono::{Duration, Utc};
use tempfile::TempDir;
use uuid::Uuid;

/// Create a fresh in-memory-like test database.
async fn test_pool() -> (DbPool, TempDir) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let pool = create_pool(&db_path).await.unwrap();
    run_migrations(&pool).await.unwrap();
    (pool, dir)
}

fn test_book(title: &str) -> Book {
    Book::new(title)
}

fn test_author(name: &str) -> Author {
    Author::new(name)
}

// ── BookRepository ──────────────────────────────────────────────

#[tokio::test]
async fn book_create_and_get() {
    let (pool, _dir) = test_pool().await;
    let book = test_book("Dune");

    BookRepository::create(&pool, &book).await.unwrap();
    let fetched = BookRepository::get_by_id(&pool, book.id).await.unwrap();

    assert_eq!(fetched.id, book.id);
    assert_eq!(fetched.title, "Dune");
    assert_eq!(fetched.sort_title, "Dune");
    assert_eq!(fetched.metadata_status, MetadataStatus::Unidentified);
}

#[tokio::test]
async fn book_get_not_found() {
    let (pool, _dir) = test_pool().await;
    let result = BookRepository::get_by_id(&pool, Uuid::new_v4()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn book_update() {
    let (pool, _dir) = test_pool().await;
    let mut book = test_book("Dume");
    BookRepository::create(&pool, &book).await.unwrap();

    book.title = "Dune".into();
    book.sort_title = "Dune".into();
    book.description = Some("A desert planet saga".into());
    book.rating = Some(4.5);
    book.metadata_status = MetadataStatus::Identified;
    book.ingest_quality_score = 0.95;

    BookRepository::update(&pool, &book).await.unwrap();
    let fetched = BookRepository::get_by_id(&pool, book.id).await.unwrap();

    assert_eq!(fetched.title, "Dune");
    assert_eq!(fetched.description.as_deref(), Some("A desert planet saga"));
    assert!((fetched.rating.unwrap() - 4.5).abs() < f32::EPSILON);
    assert_eq!(fetched.metadata_status, MetadataStatus::Identified);
}

#[tokio::test]
async fn book_delete() {
    let (pool, _dir) = test_pool().await;
    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    BookRepository::delete(&pool, book.id).await.unwrap();
    let result = BookRepository::get_by_id(&pool, book.id).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn book_delete_not_found() {
    let (pool, _dir) = test_pool().await;
    let result = BookRepository::delete(&pool, Uuid::new_v4()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn book_resolution_queue_orders_by_requested_at_and_filters_locked_books() {
    let (pool, _dir) = test_pool().await;
    let base = Utc::now();

    let mut first = test_book("First");
    first.resolution_requested_at = base - Duration::hours(3);
    first.metadata_status = MetadataStatus::Identified;
    first.ingest_quality_score = 1.0;
    BookRepository::create(&pool, &first).await.unwrap();

    let mut locked = test_book("Locked");
    locked.resolution_requested_at = base - Duration::hours(4);
    locked.metadata_locked = true;
    BookRepository::create(&pool, &locked).await.unwrap();

    let mut done = test_book("Done");
    done.resolution_requested_at = base - Duration::hours(5);
    done.resolution_state = ResolutionState::Done;
    BookRepository::create(&pool, &done).await.unwrap();

    let mut second = test_book("Second");
    second.resolution_requested_at = base - Duration::hours(1);
    BookRepository::create(&pool, &second).await.unwrap();

    let queued = BookRepository::list_pending_resolution(&pool, 10)
        .await
        .unwrap();

    let queued_ids: Vec<Uuid> = queued.into_iter().map(|book| book.id).collect();
    assert_eq!(queued_ids, vec![first.id, second.id]);
}

#[tokio::test]
async fn book_resolution_claim_is_compare_and_set_noop_after_first_claim() {
    let (pool, _dir) = test_pool().await;
    let book = test_book("Claimed Once");
    BookRepository::create(&pool, &book).await.unwrap();

    let claimed_first = BookRepository::claim_pending_resolution(&pool, book.id)
        .await
        .unwrap();
    let claimed_second = BookRepository::claim_pending_resolution(&pool, book.id)
        .await
        .unwrap();

    assert!(claimed_first);
    assert!(!claimed_second);

    let updated = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(updated.resolution_state, ResolutionState::Running);
}

#[tokio::test]
async fn book_list_with_pagination() {
    let (pool, _dir) = test_pool().await;

    for i in 0..10 {
        let book = test_book(&format!("Book {i:02}"));
        BookRepository::create(&pool, &book).await.unwrap();
    }

    let params = PaginationParams {
        page: 1,
        per_page: 3,
        sort_by: "title".into(),
        sort_order: SortOrder::Asc,
    };
    let result = BookRepository::list(&pool, &params, &BookFilter::default())
        .await
        .unwrap();

    assert_eq!(result.total, 10);
    assert_eq!(result.items.len(), 3);
    assert_eq!(result.total_pages, 4);
    assert_eq!(result.items[0].title, "Book 00");
    assert_eq!(result.items[2].title, "Book 02");

    // Page 4 should have 1 item
    let params_last = PaginationParams {
        page: 4,
        per_page: 3,
        sort_by: "title".into(),
        sort_order: SortOrder::Asc,
    };
    let result_last = BookRepository::list(&pool, &params_last, &BookFilter::default())
        .await
        .unwrap();
    assert_eq!(result_last.items.len(), 1);
    assert_eq!(result_last.items[0].title, "Book 09");
}

#[tokio::test]
async fn book_list_filter_by_status() {
    let (pool, _dir) = test_pool().await;

    let mut book1 = test_book("Identified Book");
    book1.metadata_status = MetadataStatus::Identified;
    BookRepository::create(&pool, &book1).await.unwrap();

    let book2 = test_book("Unidentified Book");
    BookRepository::create(&pool, &book2).await.unwrap();

    let filter = BookFilter {
        status: Some(MetadataStatus::Identified),
        trusted: None,
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Identified Book");
}

#[tokio::test]
async fn book_search_fts() {
    let (pool, _dir) = test_pool().await;

    let mut book1 = test_book("Dune");
    book1.description = Some("A desert planet saga about spice".into());
    BookRepository::create(&pool, &book1).await.unwrap();

    let book2 = test_book("Foundation");
    BookRepository::create(&pool, &book2).await.unwrap();

    // Search by title
    let result = BookRepository::search(&pool, "dune", &PaginationParams::default())
        .await
        .unwrap();
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Dune");

    // Search by description
    let result = BookRepository::search(&pool, "spice", &PaginationParams::default())
        .await
        .unwrap();
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Dune");
}

#[tokio::test]
async fn book_search_by_author_name() {
    let (pool, _dir) = test_pool().await;

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let author = test_author("Frank Herbert");
    AuthorRepository::create(&pool, &author).await.unwrap();

    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let result = BookRepository::search(&pool, "herbert", &PaginationParams::default())
        .await
        .unwrap();
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Dune");
}

#[tokio::test]
async fn book_with_relations() {
    let (pool, _dir) = test_pool().await;

    // Create publisher
    let publisher = Publisher::new("Ace Books");
    sqlx::query("INSERT INTO publishers (id, name) VALUES (?, ?)")
        .bind(publisher.id.to_string())
        .bind(&publisher.name)
        .execute(&pool)
        .await
        .unwrap();

    // Create book with publisher
    let mut book = test_book("Dune");
    book.publisher_id = Some(publisher.id);
    BookRepository::create(&pool, &book).await.unwrap();

    // Create and link author
    let author = test_author("Frank Herbert");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    // Create and link series
    let series = Series::new("Dune Chronicles");
    SeriesRepository::create(&pool, &series).await.unwrap();
    BookRepository::add_series(&pool, book.id, series.id, Some(1.0))
        .await
        .unwrap();

    // Create and link tag
    let tag = Tag::with_category("science fiction", "genre");
    TagRepository::create(&pool, &tag).await.unwrap();
    BookRepository::add_tag(&pool, book.id, tag.id)
        .await
        .unwrap();

    // Create book file
    let file = BookFile::new(
        book.id,
        BookFormat::Epub,
        "d/dune.epub",
        1_000_000,
        "abc123",
        None,
    );
    BookFileRepository::create(&pool, &file).await.unwrap();

    // Create identifier
    let identifier = Identifier::new(
        book.id,
        IdentifierType::Isbn13,
        "978-0-441-17271-9",
        MetadataSource::Embedded,
        0.95,
    );
    IdentifierRepository::create(&pool, &identifier)
        .await
        .unwrap();

    // Fetch with all relations
    let full = BookRepository::get_with_relations(&pool, book.id)
        .await
        .unwrap();

    assert_eq!(full.book.title, "Dune");
    assert_eq!(full.publisher_name.as_deref(), Some("Ace Books"));
    assert_eq!(full.authors.len(), 1);
    assert_eq!(full.authors[0].author.name, "Frank Herbert");
    assert_eq!(full.authors[0].role, "author");
    assert_eq!(full.series.len(), 1);
    assert_eq!(full.series[0].series.name, "Dune Chronicles");
    assert!((full.series[0].position.unwrap() - 1.0).abs() < f64::EPSILON);
    assert_eq!(full.files.len(), 1);
    assert_eq!(full.files[0].format, BookFormat::Epub);
    assert_eq!(full.identifiers.len(), 1);
    assert_eq!(full.identifiers[0].value, "978-0-441-17271-9");
    assert_eq!(full.tags.len(), 1);
    assert_eq!(full.tags[0].name, "science fiction");
}

#[tokio::test]
async fn book_cascade_delete() {
    let (pool, _dir) = test_pool().await;

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let author = test_author("Frank Herbert");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let file = BookFile::new(book.id, BookFormat::Epub, "path.epub", 100, "hash1", None);
    BookFileRepository::create(&pool, &file).await.unwrap();

    // Delete book — should cascade to book_authors and book_files
    BookRepository::delete(&pool, book.id).await.unwrap();

    let files = BookFileRepository::get_by_book_id(&pool, book.id)
        .await
        .unwrap();
    assert!(files.is_empty());

    // Author should still exist (only the junction row is deleted)
    let author_still = AuthorRepository::get_by_id(&pool, author.id).await;
    assert!(author_still.is_ok());
}

// ── AuthorRepository ────────────────────────────────────────────

#[tokio::test]
async fn author_crud() {
    let (pool, _dir) = test_pool().await;
    let author = test_author("Frank Herbert");

    AuthorRepository::create(&pool, &author).await.unwrap();
    let fetched = AuthorRepository::get_by_id(&pool, author.id).await.unwrap();
    assert_eq!(fetched.name, "Frank Herbert");
    assert_eq!(fetched.sort_name, "Herbert, Frank");

    // Update
    let mut updated = fetched;
    updated.name = "Frank Patrick Herbert".into();
    updated.sort_name = "Herbert, Frank Patrick".into();
    AuthorRepository::update(&pool, &updated).await.unwrap();

    let refetched = AuthorRepository::get_by_id(&pool, author.id).await.unwrap();
    assert_eq!(refetched.name, "Frank Patrick Herbert");

    // Delete
    AuthorRepository::delete(&pool, author.id).await.unwrap();
    assert!(AuthorRepository::get_by_id(&pool, author.id).await.is_err());
}

#[tokio::test]
async fn author_find_by_name() {
    let (pool, _dir) = test_pool().await;
    let author = test_author("Frank Herbert");
    AuthorRepository::create(&pool, &author).await.unwrap();

    let found = AuthorRepository::find_by_name(&pool, "Frank Herbert")
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, author.id);

    // Case insensitive
    let found_ci = AuthorRepository::find_by_name(&pool, "frank herbert")
        .await
        .unwrap();
    assert!(found_ci.is_some());

    // Not found
    let not_found = AuthorRepository::find_by_name(&pool, "Isaac Asimov")
        .await
        .unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn author_list_sorted() {
    let (pool, _dir) = test_pool().await;

    for name in ["Tolkien", "Asimov", "Herbert"] {
        AuthorRepository::create(&pool, &test_author(name))
            .await
            .unwrap();
    }

    let params = PaginationParams {
        sort_by: "sort_name".into(),
        sort_order: SortOrder::Asc,
        ..PaginationParams::default()
    };
    let result = AuthorRepository::list(&pool, &params).await.unwrap();

    assert_eq!(result.total, 3);
    assert_eq!(result.items[0].author.name, "Asimov");
    assert_eq!(result.items[1].author.name, "Herbert");
    assert_eq!(result.items[2].author.name, "Tolkien");
}

// ── SeriesRepository ────────────────────────────────────────────

#[tokio::test]
async fn series_crud() {
    let (pool, _dir) = test_pool().await;

    let mut series = Series::new("Discworld");
    SeriesRepository::create(&pool, &series).await.unwrap();

    let fetched = SeriesRepository::get_by_id(&pool, series.id).await.unwrap();
    assert_eq!(fetched.name, "Discworld");

    series.description = Some("Terry Pratchett's satirical fantasy series".into());
    SeriesRepository::update(&pool, &series).await.unwrap();

    let refetched = SeriesRepository::get_by_id(&pool, series.id).await.unwrap();
    assert_eq!(
        refetched.description.as_deref(),
        Some("Terry Pratchett's satirical fantasy series")
    );

    SeriesRepository::delete(&pool, series.id).await.unwrap();
    assert!(SeriesRepository::get_by_id(&pool, series.id).await.is_err());
}

#[tokio::test]
async fn series_find_or_create_new() {
    let (pool, _dir) = test_pool().await;

    let series = SeriesRepository::find_or_create(&pool, "Discworld")
        .await
        .unwrap();
    assert_eq!(series.name, "Discworld");

    // Verify it actually exists in the database
    let fetched = SeriesRepository::get_by_id(&pool, series.id).await.unwrap();
    assert_eq!(fetched.name, "Discworld");
}

#[tokio::test]
async fn series_find_or_create_dedup_case_insensitive() {
    let (pool, _dir) = test_pool().await;

    let s1 = SeriesRepository::find_or_create(&pool, "Harry Potter")
        .await
        .unwrap();
    let s2 = SeriesRepository::find_or_create(&pool, "harry potter")
        .await
        .unwrap();
    let s3 = SeriesRepository::find_or_create(&pool, "HARRY POTTER")
        .await
        .unwrap();

    assert_eq!(s1.id, s2.id, "same series regardless of case");
    assert_eq!(s1.id, s3.id, "same series regardless of case");
    // Original casing is preserved
    assert_eq!(s1.name, "Harry Potter");
}

#[tokio::test]
async fn series_find_or_create_distinct_names() {
    let (pool, _dir) = test_pool().await;

    let s1 = SeriesRepository::find_or_create(&pool, "Discworld")
        .await
        .unwrap();
    let s2 = SeriesRepository::find_or_create(&pool, "Dune Chronicles")
        .await
        .unwrap();

    assert_ne!(s1.id, s2.id);
}

#[tokio::test]
async fn book_update_series_position() {
    let (pool, _dir) = test_pool().await;

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();
    let series = Series::new("Dune Chronicles");
    SeriesRepository::create(&pool, &series).await.unwrap();

    // Add with no position
    BookRepository::add_series(&pool, book.id, series.id, None)
        .await
        .unwrap();
    let rel = BookRepository::get_with_relations(&pool, book.id)
        .await
        .unwrap();
    assert!(rel.series[0].position.is_none());

    // Backfill position
    BookRepository::update_series_position(&pool, book.id, series.id, Some(3.0))
        .await
        .unwrap();
    let rel = BookRepository::get_with_relations(&pool, book.id)
        .await
        .unwrap();
    assert!((rel.series[0].position.unwrap() - 3.0).abs() < f64::EPSILON);
}

// ── BookFileRepository ──────────────────────────────────────────

#[tokio::test]
async fn book_file_create_and_query() {
    let (pool, _dir) = test_pool().await;

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let file = BookFile::new(
        book.id,
        BookFormat::Epub,
        "d/dune.epub",
        500_000,
        "sha256hash",
        None,
    );
    BookFileRepository::create(&pool, &file).await.unwrap();

    // Get by book_id
    let files = BookFileRepository::get_by_book_id(&pool, book.id)
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].format, BookFormat::Epub);

    // Get by hash
    let by_hash = BookFileRepository::get_by_hash(&pool, "sha256hash")
        .await
        .unwrap();
    assert!(by_hash.is_some());
    assert_eq!(by_hash.unwrap().id, file.id);

    // Hash not found
    let not_found = BookFileRepository::get_by_hash(&pool, "nonexistent")
        .await
        .unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn book_file_duplicate_hash_rejected() {
    let (pool, _dir) = test_pool().await;

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let file1 = BookFile::new(
        book.id,
        BookFormat::Epub,
        "path1.epub",
        100,
        "samehash",
        None,
    );
    BookFileRepository::create(&pool, &file1).await.unwrap();

    let file2 = BookFile::new(book.id, BookFormat::Pdf, "path2.pdf", 200, "samehash", None);
    let result = BookFileRepository::create(&pool, &file2).await;
    assert!(result.is_err());
}

// ── IdentifierRepository ────────────────────────────────────────

#[tokio::test]
async fn identifier_create_and_query() {
    let (pool, _dir) = test_pool().await;

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let isbn = Identifier::new(
        book.id,
        IdentifierType::Isbn13,
        "978-0-441-17271-9",
        MetadataSource::Embedded,
        0.95,
    );
    IdentifierRepository::create(&pool, &isbn).await.unwrap();

    let asin = Identifier::new(
        book.id,
        IdentifierType::Asin,
        "B000FA5ZEG",
        MetadataSource::Provider("Amazon".into()),
        0.8,
    );
    IdentifierRepository::create(&pool, &asin).await.unwrap();

    // Get by book
    let idents = IdentifierRepository::get_by_book_id(&pool, book.id)
        .await
        .unwrap();
    assert_eq!(idents.len(), 2);

    // Find by value
    let found = IdentifierRepository::find_by_value(&pool, "isbn13", "978-0-441-17271-9")
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].book_id, book.id);

    // Verify source roundtrip
    let asin_found = idents
        .iter()
        .find(|i| i.identifier_type == IdentifierType::Asin)
        .unwrap();
    assert!(matches!(&asin_found.source, MetadataSource::Provider(name) if name == "Amazon"));
}

// ── TagRepository ───────────────────────────────────────────────

#[tokio::test]
async fn tag_create_and_find_or_create() {
    let (pool, _dir) = test_pool().await;

    // find_or_create should create new
    let tag = TagRepository::find_or_create(&pool, "science fiction", Some("genre"))
        .await
        .unwrap();
    assert_eq!(tag.name, "science fiction");
    assert_eq!(tag.category.as_deref(), Some("genre"));

    // find_or_create same name+category should return existing
    let tag2 = TagRepository::find_or_create(&pool, "science fiction", Some("genre"))
        .await
        .unwrap();
    assert_eq!(tag2.id, tag.id);

    // Different category creates new
    let tag3 = TagRepository::find_or_create(&pool, "science fiction", None)
        .await
        .unwrap();
    assert_ne!(tag3.id, tag.id);
}

#[tokio::test]
async fn tag_list() {
    let (pool, _dir) = test_pool().await;

    TagRepository::create(&pool, &Tag::new("fantasy"))
        .await
        .unwrap();
    TagRepository::create(&pool, &Tag::new("science fiction"))
        .await
        .unwrap();
    TagRepository::create(&pool, &Tag::new("horror"))
        .await
        .unwrap();

    let params = PaginationParams {
        sort_by: "name".into(),
        sort_order: SortOrder::Asc,
        ..PaginationParams::default()
    };
    let result = TagRepository::list(&pool, &params).await.unwrap();

    assert_eq!(result.total, 3);
    assert_eq!(result.items[0].tag.name, "fantasy");
    assert_eq!(result.items[0].book_count, 0);
    assert_eq!(result.items[1].tag.name, "horror");
    assert_eq!(result.items[2].tag.name, "science fiction");
}

#[tokio::test]
async fn tag_list_with_book_counts() {
    let (pool, _dir) = test_pool().await;

    let tag_a = Tag::new("fantasy");
    let tag_b = Tag::new("scifi");
    let tag_c = Tag::new("romance");
    TagRepository::create(&pool, &tag_a).await.unwrap();
    TagRepository::create(&pool, &tag_b).await.unwrap();
    TagRepository::create(&pool, &tag_c).await.unwrap();

    let book1 = test_book("Book One");
    let book2 = test_book("Book Two");
    BookRepository::create(&pool, &book1).await.unwrap();
    BookRepository::create(&pool, &book2).await.unwrap();

    // `fantasy` gets 2 books, `scifi` gets 1, `romance` gets 0
    BookRepository::add_tag(&pool, book1.id, tag_a.id)
        .await
        .unwrap();
    BookRepository::add_tag(&pool, book2.id, tag_a.id)
        .await
        .unwrap();
    BookRepository::add_tag(&pool, book1.id, tag_b.id)
        .await
        .unwrap();

    let params = PaginationParams {
        sort_by: "name".into(),
        sort_order: SortOrder::Asc,
        ..PaginationParams::default()
    };
    let result = TagRepository::list(&pool, &params).await.unwrap();

    assert_eq!(result.total, 3);
    assert_eq!(result.items[0].tag.name, "fantasy");
    assert_eq!(result.items[0].book_count, 2);
    assert_eq!(result.items[1].tag.name, "romance");
    assert_eq!(result.items[1].book_count, 0);
    assert_eq!(result.items[2].tag.name, "scifi");
    assert_eq!(result.items[2].book_count, 1);
}

#[tokio::test]
async fn tag_list_sort_by_book_count() {
    let (pool, _dir) = test_pool().await;

    let tag_a = Tag::new("alpha");
    let tag_b = Tag::new("beta");
    let tag_c = Tag::new("gamma");
    TagRepository::create(&pool, &tag_a).await.unwrap();
    TagRepository::create(&pool, &tag_b).await.unwrap();
    TagRepository::create(&pool, &tag_c).await.unwrap();

    let book1 = test_book("B1");
    let book2 = test_book("B2");
    let book3 = test_book("B3");
    BookRepository::create(&pool, &book1).await.unwrap();
    BookRepository::create(&pool, &book2).await.unwrap();
    BookRepository::create(&pool, &book3).await.unwrap();

    // gamma=3, alpha=1, beta=0
    BookRepository::add_tag(&pool, book1.id, tag_c.id)
        .await
        .unwrap();
    BookRepository::add_tag(&pool, book2.id, tag_c.id)
        .await
        .unwrap();
    BookRepository::add_tag(&pool, book3.id, tag_c.id)
        .await
        .unwrap();
    BookRepository::add_tag(&pool, book1.id, tag_a.id)
        .await
        .unwrap();

    // DESC: gamma(3), alpha(1), beta(0)
    let params = PaginationParams {
        sort_by: "book_count".into(),
        sort_order: SortOrder::Desc,
        ..PaginationParams::default()
    };
    let result = TagRepository::list(&pool, &params).await.unwrap();
    assert_eq!(result.items[0].tag.name, "gamma");
    assert_eq!(result.items[0].book_count, 3);
    assert_eq!(result.items[1].tag.name, "alpha");
    assert_eq!(result.items[1].book_count, 1);
    assert_eq!(result.items[2].tag.name, "beta");
    assert_eq!(result.items[2].book_count, 0);

    // ASC: beta(0), alpha(1), gamma(3)
    let params_asc = PaginationParams {
        sort_by: "book_count".into(),
        sort_order: SortOrder::Asc,
        ..PaginationParams::default()
    };
    let result_asc = TagRepository::list(&pool, &params_asc).await.unwrap();
    assert_eq!(result_asc.items[0].tag.name, "beta");
    assert_eq!(result_asc.items[1].tag.name, "alpha");
    assert_eq!(result_asc.items[2].tag.name, "gamma");
}

#[tokio::test]
async fn tag_search_with_book_counts() {
    let (pool, _dir) = test_pool().await;

    let tag_a = Tag::with_category("dark fantasy", "genre");
    let tag_b = Tag::with_category("urban fantasy", "genre");
    let tag_c = Tag::new("romance");
    TagRepository::create(&pool, &tag_a).await.unwrap();
    TagRepository::create(&pool, &tag_b).await.unwrap();
    TagRepository::create(&pool, &tag_c).await.unwrap();

    let book1 = test_book("B1");
    BookRepository::create(&pool, &book1).await.unwrap();
    BookRepository::add_tag(&pool, book1.id, tag_a.id)
        .await
        .unwrap();

    let params = PaginationParams {
        sort_by: "name".into(),
        sort_order: SortOrder::Asc,
        ..PaginationParams::default()
    };

    // Search by name
    let result = TagRepository::search(&pool, Some("fantasy"), None, &params)
        .await
        .unwrap();
    assert_eq!(result.total, 2);
    assert_eq!(result.items[0].tag.name, "dark fantasy");
    assert_eq!(result.items[0].book_count, 1);
    assert_eq!(result.items[1].tag.name, "urban fantasy");
    assert_eq!(result.items[1].book_count, 0);

    // Filter by category
    let result2 = TagRepository::search(&pool, None, Some("genre"), &params)
        .await
        .unwrap();
    assert_eq!(result2.total, 2);

    // Search + category
    let result3 = TagRepository::search(&pool, Some("dark"), Some("genre"), &params)
        .await
        .unwrap();
    assert_eq!(result3.total, 1);
    assert_eq!(result3.items[0].tag.name, "dark fantasy");
}

#[tokio::test]
async fn tag_list_categories() {
    let (pool, _dir) = test_pool().await;

    TagRepository::create(&pool, &Tag::with_category("fantasy", "genre"))
        .await
        .unwrap();
    TagRepository::create(&pool, &Tag::with_category("hardcover", "format"))
        .await
        .unwrap();
    TagRepository::create(&pool, &Tag::new("uncategorized"))
        .await
        .unwrap();

    let cats = TagRepository::list_categories(&pool).await.unwrap();
    assert_eq!(cats, vec!["format", "genre"]);
}

// ── Filter by author and series ─────────────────────────────────

#[tokio::test]
async fn book_list_filter_by_author() {
    let (pool, _dir) = test_pool().await;

    let author1 = test_author("Frank Herbert");
    AuthorRepository::create(&pool, &author1).await.unwrap();
    let author2 = test_author("Isaac Asimov");
    AuthorRepository::create(&pool, &author2).await.unwrap();

    let book1 = test_book("Dune");
    BookRepository::create(&pool, &book1).await.unwrap();
    BookRepository::add_author(&pool, book1.id, author1.id, "author", 0)
        .await
        .unwrap();

    let book2 = test_book("Foundation");
    BookRepository::create(&pool, &book2).await.unwrap();
    BookRepository::add_author(&pool, book2.id, author2.id, "author", 0)
        .await
        .unwrap();

    let filter = BookFilter {
        author_id: Some(author1.id.to_string()),
        trusted: None,
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Dune");
}

#[tokio::test]
async fn book_list_filter_by_format() {
    let (pool, _dir) = test_pool().await;

    let book1 = test_book("EPUB Book");
    BookRepository::create(&pool, &book1).await.unwrap();
    let file1 = BookFile::new(book1.id, BookFormat::Epub, "a.epub", 100, "h1", None);
    BookFileRepository::create(&pool, &file1).await.unwrap();

    let book2 = test_book("PDF Book");
    BookRepository::create(&pool, &book2).await.unwrap();
    let file2 = BookFile::new(book2.id, BookFormat::Pdf, "b.pdf", 200, "h2", None);
    BookFileRepository::create(&pool, &file2).await.unwrap();

    let filter = BookFilter {
        format: Some(BookFormat::Epub),
        trusted: None,
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "EPUB Book");
}

// ── StatsRepository ─────────────────────────────────────────────

#[tokio::test]
async fn stats_repository_returns_library_usage_and_db_stats() {
    let (pool, _dir) = test_pool().await;

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();
    let file = BookFile::new(
        book.id,
        BookFormat::Epub,
        "dune.epub",
        1_024,
        "hash-dune",
        None,
    );
    BookFileRepository::create(&pool, &file).await.unwrap();

    sqlx::query(
        "INSERT INTO tasks (id, task_type, payload, status, progress) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind("import_file")
    .bind("{}")
    .bind("completed")
    .bind(100_i64)
    .execute(&pool)
    .await
    .unwrap();

    // Use two books to satisfy duplicate link constraints.
    let other = test_book("Dune Copy");
    BookRepository::create(&pool, &other).await.unwrap();
    sqlx::query(
        "INSERT INTO duplicate_links (id, book_id_a, book_id_b, detection_method, confidence, status) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book.id.to_string())
    .bind(other.id.to_string())
    .bind("user")
    .bind(1.0_f32)
    .bind("pending")
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO identification_candidates (id, book_id, provider_name, score, metadata, status) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book.id.to_string())
    .bind("open_library")
    .bind(0.9_f32)
    .bind("{}")
    .bind("pending")
    .execute(&pool)
    .await
    .unwrap();

    let library = StatsRepository::library_overview(&pool).await.unwrap();
    assert_eq!(library.books, 2);
    assert_eq!(library.files, 1);
    assert_eq!(library.total_file_size, 1_024);

    let formats = StatsRepository::files_by_format(&pool).await.unwrap();
    assert_eq!(formats.len(), 1);
    assert_eq!(formats[0].format, "epub");
    assert_eq!(formats[0].file_count, 1);

    let task_overview = StatsRepository::task_overview(&pool).await.unwrap();
    assert_eq!(task_overview.total, 1);
    assert_eq!(task_overview.last_24h, 1);

    let pending_duplicates = StatsRepository::pending_duplicate_count(&pool)
        .await
        .unwrap();
    assert_eq!(pending_duplicates, 1);

    let pending_candidates = StatsRepository::pending_candidate_count(&pool)
        .await
        .unwrap();
    assert_eq!(pending_candidates, 1);

    let pragma = StatsRepository::db_pragma_stats(&pool).await.unwrap();
    assert!(pragma.page_size > 0);
    assert!(pragma.page_count > 0);

    let objects = StatsRepository::db_object_stats(&pool).await.unwrap();
    assert!(!objects.objects.is_empty());
    assert!(objects.objects.iter().any(|entry| entry.name == "books"));
}

#[tokio::test]
async fn stats_pending_candidate_count_ignores_historical_runs() {
    let (pool, _dir) = test_pool().await;
    let book = test_book("Children of Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let mut historical_run = ResolutionRun::new(
        book.id,
        "import",
        serde_json::json!({"title":"Children of Dune"}),
    );
    historical_run.state = archivis_core::models::ResolutionRunState::Done;
    historical_run.started_at = Utc::now() - Duration::minutes(10);
    historical_run.finished_at = Some(Utc::now() - Duration::minutes(9));
    ResolutionRunRepository::create(&pool, &historical_run)
        .await
        .unwrap();

    let mut historical_candidate = IdentificationCandidate::new(
        book.id,
        "historical",
        0.6,
        serde_json::json!({"title":"Children of Dune"}),
        vec!["fallback".into()],
    );
    historical_candidate.run_id = Some(historical_run.id);
    CandidateRepository::create(&pool, &historical_candidate)
        .await
        .unwrap();

    let current_run = ResolutionRunRepository::start(
        &pool,
        book.id,
        "manual_refresh",
        serde_json::json!({"title":"Children of Dune"}),
        "running",
    )
    .await
    .unwrap();

    let mut current_candidate = IdentificationCandidate::new(
        book.id,
        "current",
        0.9,
        serde_json::json!({"title":"Children of Dune"}),
        vec!["title_match".into()],
    );
    current_candidate.run_id = Some(current_run.id);
    CandidateRepository::create(&pool, &current_candidate)
        .await
        .unwrap();

    let pending_candidates = StatsRepository::pending_candidate_count(&pool)
        .await
        .unwrap();
    assert_eq!(pending_candidates, 1);
}

// ── Helper: create a test user ─────────────────────────────────

fn test_user(username: &str) -> User {
    User::new(username.into(), "hashed_pw".into(), UserRole::User)
}

// ── ReadingProgressRepository ──────────────────────────────────

#[tokio::test]
async fn upsert_creates_new_progress() {
    let (pool, _dir) = test_pool().await;

    let user = test_user("reader");
    UserRepository::create(&pool, &user).await.unwrap();

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let file = BookFile::new(
        book.id,
        BookFormat::Epub,
        "dune.epub",
        100_000,
        "hash1",
        None,
    );
    BookFileRepository::create(&pool, &file).await.unwrap();

    let progress = ReadingProgressRepository::upsert(
        &pool,
        user.id,
        file.id,
        book.id,
        Some("epubcfi(/6/4)"),
        0.42,
        None,
        Some(&serde_json::json!({"fontSize": 16})),
    )
    .await
    .unwrap();

    assert_eq!(progress.user_id, user.id);
    assert_eq!(progress.book_id, book.id);
    assert_eq!(progress.book_file_id, file.id);
    assert_eq!(progress.location.as_deref(), Some("epubcfi(/6/4)"));
    assert!((progress.progress - 0.42).abs() < f64::EPSILON);
    assert!(progress.device_id.is_none());
    assert!(progress.preferences.is_some());
}

#[tokio::test]
async fn upsert_updates_existing_progress() {
    let (pool, _dir) = test_pool().await;

    let user = test_user("reader");
    UserRepository::create(&pool, &user).await.unwrap();

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let file = BookFile::new(
        book.id,
        BookFormat::Epub,
        "dune.epub",
        100_000,
        "hash1",
        None,
    );
    BookFileRepository::create(&pool, &file).await.unwrap();

    // First upsert
    let p1 = ReadingProgressRepository::upsert(
        &pool,
        user.id,
        file.id,
        book.id,
        Some("epubcfi(/6/4)"),
        0.10,
        None,
        None,
    )
    .await
    .unwrap();

    // Second upsert (same user, file, device=NULL)
    let p2 = ReadingProgressRepository::upsert(
        &pool,
        user.id,
        file.id,
        book.id,
        Some("epubcfi(/6/10)"),
        0.50,
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(p1.id, p2.id, "upsert should update the same row");
    assert_eq!(p2.location.as_deref(), Some("epubcfi(/6/10)"));
    assert!((p2.progress - 0.50).abs() < f64::EPSILON);
}

#[tokio::test]
async fn upsert_distinct_device_ids() {
    let (pool, _dir) = test_pool().await;

    let user = test_user("reader");
    UserRepository::create(&pool, &user).await.unwrap();

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let file = BookFile::new(
        book.id,
        BookFormat::Epub,
        "dune.epub",
        100_000,
        "hash1",
        None,
    );
    BookFileRepository::create(&pool, &file).await.unwrap();

    // Web browser (NULL device)
    let p1 = ReadingProgressRepository::upsert(
        &pool,
        user.id,
        file.id,
        book.id,
        Some("epubcfi(/6/4)"),
        0.10,
        None,
        None,
    )
    .await
    .unwrap();

    // KOReader device
    let p2 = ReadingProgressRepository::upsert(
        &pool,
        user.id,
        file.id,
        book.id,
        Some("epubcfi(/6/20)"),
        0.80,
        Some("koreader-1"),
        None,
    )
    .await
    .unwrap();

    assert_ne!(
        p1.id, p2.id,
        "different device_ids should create separate rows"
    );
    assert!(p1.device_id.is_none());
    assert_eq!(p2.device_id.as_deref(), Some("koreader-1"));
}

#[tokio::test]
async fn get_for_book_returns_most_recent() {
    let (pool, _dir) = test_pool().await;

    let user = test_user("reader");
    UserRepository::create(&pool, &user).await.unwrap();

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let file1 = BookFile::new(
        book.id,
        BookFormat::Epub,
        "dune.epub",
        100_000,
        "hash1",
        None,
    );
    BookFileRepository::create(&pool, &file1).await.unwrap();

    let file2 = BookFile::new(book.id, BookFormat::Pdf, "dune.pdf", 200_000, "hash2", None);
    BookFileRepository::create(&pool, &file2).await.unwrap();

    // Progress on file1 first
    ReadingProgressRepository::upsert(
        &pool,
        user.id,
        file1.id,
        book.id,
        Some("epubcfi(/6/4)"),
        0.10,
        None,
        None,
    )
    .await
    .unwrap();

    // Progress on file2 later (more recent)
    ReadingProgressRepository::upsert(
        &pool,
        user.id,
        file2.id,
        book.id,
        Some("page:5"),
        0.50,
        None,
        None,
    )
    .await
    .unwrap();

    let most_recent = ReadingProgressRepository::get_for_book(&pool, user.id, book.id)
        .await
        .unwrap()
        .expect("should find progress");

    assert_eq!(
        most_recent.book_file_id, file2.id,
        "should return most recently updated"
    );
    assert!((most_recent.progress - 0.50).abs() < f64::EPSILON);
}

#[tokio::test]
async fn get_for_file_with_null_device() {
    let (pool, _dir) = test_pool().await;

    let user = test_user("reader");
    UserRepository::create(&pool, &user).await.unwrap();

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let file = BookFile::new(
        book.id,
        BookFormat::Epub,
        "dune.epub",
        100_000,
        "hash1",
        None,
    );
    BookFileRepository::create(&pool, &file).await.unwrap();

    // Insert with NULL device_id
    ReadingProgressRepository::upsert(
        &pool,
        user.id,
        file.id,
        book.id,
        Some("epubcfi(/6/4)"),
        0.42,
        None,
        None,
    )
    .await
    .unwrap();

    // Look up with NULL device_id
    let found = ReadingProgressRepository::get_for_file(&pool, user.id, file.id, None)
        .await
        .unwrap();
    assert!(found.is_some(), "should find progress with NULL device_id");
    assert!(found.unwrap().device_id.is_none());

    // Look up with a named device should NOT find it
    let not_found =
        ReadingProgressRepository::get_for_file(&pool, user.id, file.id, Some("koreader"))
            .await
            .unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn list_recent_respects_limit() {
    let (pool, _dir) = test_pool().await;

    let user = test_user("reader");
    UserRepository::create(&pool, &user).await.unwrap();

    // Create 5 books with progress
    for i in 0..5 {
        let book = test_book(&format!("Book {i}"));
        BookRepository::create(&pool, &book).await.unwrap();

        let file = BookFile::new(
            book.id,
            BookFormat::Epub,
            format!("book{i}.epub"),
            100_000,
            format!("hash{i}"),
            None,
        );
        BookFileRepository::create(&pool, &file).await.unwrap();

        ReadingProgressRepository::upsert(
            &pool,
            user.id,
            file.id,
            book.id,
            None,
            f64::from(i) * 0.2,
            None,
            None,
        )
        .await
        .unwrap();
    }

    let recent = ReadingProgressRepository::list_recent(&pool, user.id, 3)
        .await
        .unwrap();

    assert_eq!(recent.len(), 3, "should respect limit");
    // Results should be ordered by updated_at DESC (most recent first)
    assert!(recent[0].updated_at >= recent[1].updated_at);
    assert!(recent[1].updated_at >= recent[2].updated_at);
}

#[tokio::test]
async fn delete_for_book_removes_all() {
    let (pool, _dir) = test_pool().await;

    let user = test_user("reader");
    UserRepository::create(&pool, &user).await.unwrap();

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let file = BookFile::new(
        book.id,
        BookFormat::Epub,
        "dune.epub",
        100_000,
        "hash1",
        None,
    );
    BookFileRepository::create(&pool, &file).await.unwrap();

    ReadingProgressRepository::upsert(
        &pool,
        user.id,
        file.id,
        book.id,
        Some("epubcfi(/6/4)"),
        0.42,
        None,
        None,
    )
    .await
    .unwrap();

    let rows = ReadingProgressRepository::delete_for_book(&pool, user.id, book.id)
        .await
        .unwrap();
    assert_eq!(rows, 1);

    let found = ReadingProgressRepository::get_for_book(&pool, user.id, book.id)
        .await
        .unwrap();
    assert!(found.is_none(), "progress should be deleted");
}

// ── BookmarkRepository ─────────────────────────────────────────

#[tokio::test]
async fn bookmark_create_and_list() {
    let (pool, _dir) = test_pool().await;

    let user = test_user("reader");
    UserRepository::create(&pool, &user).await.unwrap();

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let file = BookFile::new(
        book.id,
        BookFormat::Epub,
        "dune.epub",
        100_000,
        "hash1",
        None,
    );
    BookFileRepository::create(&pool, &file).await.unwrap();

    let bm1 = Bookmark {
        id: Uuid::new_v4(),
        user_id: user.id,
        book_id: book.id,
        book_file_id: file.id,
        location: "epubcfi(/6/4)".into(),
        label: Some("Chapter 1".into()),
        excerpt: Some("A beginning is the time...".into()),
        position: 0.10,
        created_at: chrono::Utc::now(),
    };

    let bm2 = Bookmark {
        id: Uuid::new_v4(),
        user_id: user.id,
        book_id: book.id,
        book_file_id: file.id,
        location: "epubcfi(/6/20)".into(),
        label: Some("Chapter 5".into()),
        excerpt: None,
        position: 0.50,
        created_at: chrono::Utc::now(),
    };

    BookmarkRepository::create(&pool, &bm1).await.unwrap();
    BookmarkRepository::create(&pool, &bm2).await.unwrap();

    let bookmarks = BookmarkRepository::list_for_file(&pool, user.id, file.id)
        .await
        .unwrap();

    assert_eq!(bookmarks.len(), 2);
    // Ordered by position ASC
    assert!((bookmarks[0].position - 0.10).abs() < f64::EPSILON);
    assert!((bookmarks[1].position - 0.50).abs() < f64::EPSILON);
    assert_eq!(bookmarks[0].label.as_deref(), Some("Chapter 1"));
}

#[tokio::test]
async fn bookmark_delete_ownership_check() {
    let (pool, _dir) = test_pool().await;

    let user1 = test_user("reader1");
    UserRepository::create(&pool, &user1).await.unwrap();

    let user2 = test_user("reader2");
    UserRepository::create(&pool, &user2).await.unwrap();

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let file = BookFile::new(
        book.id,
        BookFormat::Epub,
        "dune.epub",
        100_000,
        "hash1",
        None,
    );
    BookFileRepository::create(&pool, &file).await.unwrap();

    let bm = Bookmark {
        id: Uuid::new_v4(),
        user_id: user1.id,
        book_id: book.id,
        book_file_id: file.id,
        location: "epubcfi(/6/4)".into(),
        label: None,
        excerpt: None,
        position: 0.10,
        created_at: chrono::Utc::now(),
    };
    BookmarkRepository::create(&pool, &bm).await.unwrap();

    // user2 tries to delete user1's bookmark
    let result = BookmarkRepository::delete(&pool, bm.id, user2.id).await;
    assert!(result.is_err(), "should fail: wrong user");

    // user1 can delete their own bookmark
    BookmarkRepository::delete(&pool, bm.id, user1.id)
        .await
        .unwrap();

    let bookmarks = BookmarkRepository::list_for_file(&pool, user1.id, file.id)
        .await
        .unwrap();
    assert!(bookmarks.is_empty());
}

#[tokio::test]
async fn bookmark_update_label() {
    let (pool, _dir) = test_pool().await;

    let user = test_user("reader");
    UserRepository::create(&pool, &user).await.unwrap();

    let book = test_book("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let file = BookFile::new(
        book.id,
        BookFormat::Epub,
        "dune.epub",
        100_000,
        "hash1",
        None,
    );
    BookFileRepository::create(&pool, &file).await.unwrap();

    let bm = Bookmark {
        id: Uuid::new_v4(),
        user_id: user.id,
        book_id: book.id,
        book_file_id: file.id,
        location: "epubcfi(/6/4)".into(),
        label: Some("Original label".into()),
        excerpt: None,
        position: 0.10,
        created_at: chrono::Utc::now(),
    };
    BookmarkRepository::create(&pool, &bm).await.unwrap();

    BookmarkRepository::update_label(&pool, bm.id, user.id, Some("Updated label"))
        .await
        .unwrap();

    let bookmarks = BookmarkRepository::list_for_file(&pool, user.id, file.id)
        .await
        .unwrap();

    assert_eq!(bookmarks.len(), 1);
    assert_eq!(bookmarks[0].label.as_deref(), Some("Updated label"));
}

// ── WatchedDirectoryRepository ─────────────────────────────────

#[tokio::test]
async fn watched_directory_create_and_get() {
    let (pool, _dir) = test_pool().await;

    let wd = WatchedDirectoryRepository::create(&pool, "/mnt/books", WatchMode::Poll, None)
        .await
        .unwrap();

    assert_eq!(wd.path, "/mnt/books");
    assert_eq!(wd.watch_mode, WatchMode::Poll);
    assert!(wd.poll_interval_secs.is_none());
    assert!(wd.enabled);
    assert!(wd.last_error.is_none());

    let fetched = WatchedDirectoryRepository::get_by_id(&pool, wd.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.id, wd.id);
    assert_eq!(fetched.path, "/mnt/books");
}

#[tokio::test]
async fn watched_directory_create_with_native_mode() {
    let (pool, _dir) = test_pool().await;

    let wd =
        WatchedDirectoryRepository::create(&pool, "/home/user/books", WatchMode::Native, Some(60))
            .await
            .unwrap();

    assert_eq!(wd.watch_mode, WatchMode::Native);
    assert_eq!(wd.poll_interval_secs, Some(60));
}

#[tokio::test]
async fn watched_directory_duplicate_path_rejected() {
    let (pool, _dir) = test_pool().await;

    WatchedDirectoryRepository::create(&pool, "/mnt/books", WatchMode::Poll, None)
        .await
        .unwrap();

    let result =
        WatchedDirectoryRepository::create(&pool, "/mnt/books", WatchMode::Native, None).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("already exists"), "unexpected error: {err}");
}

#[tokio::test]
async fn watched_directory_list_all() {
    let (pool, _dir) = test_pool().await;

    WatchedDirectoryRepository::create(&pool, "/mnt/a", WatchMode::Poll, None)
        .await
        .unwrap();
    WatchedDirectoryRepository::create(&pool, "/mnt/b", WatchMode::Native, None)
        .await
        .unwrap();

    let all = WatchedDirectoryRepository::list_all(&pool).await.unwrap();
    assert_eq!(all.len(), 2);
    // Ordered by path
    assert_eq!(all[0].path, "/mnt/a");
    assert_eq!(all[1].path, "/mnt/b");
}

#[tokio::test]
async fn watched_directory_list_enabled() {
    let (pool, _dir) = test_pool().await;

    let wd1 = WatchedDirectoryRepository::create(&pool, "/mnt/a", WatchMode::Poll, None)
        .await
        .unwrap();
    WatchedDirectoryRepository::create(&pool, "/mnt/b", WatchMode::Poll, None)
        .await
        .unwrap();

    // Disable the first one
    WatchedDirectoryRepository::update(&pool, wd1.id, None, None, Some(false))
        .await
        .unwrap();

    let enabled = WatchedDirectoryRepository::list_enabled(&pool)
        .await
        .unwrap();
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].path, "/mnt/b");
}

#[tokio::test]
async fn watched_directory_update() {
    let (pool, _dir) = test_pool().await;

    let wd = WatchedDirectoryRepository::create(&pool, "/mnt/books", WatchMode::Poll, None)
        .await
        .unwrap();

    let updated = WatchedDirectoryRepository::update(
        &pool,
        wd.id,
        Some(WatchMode::Native),
        Some(Some(45)),
        Some(false),
    )
    .await
    .unwrap();

    assert_eq!(updated.watch_mode, WatchMode::Native);
    assert_eq!(updated.poll_interval_secs, Some(45));
    assert!(!updated.enabled);
}

#[tokio::test]
async fn watched_directory_update_partial() {
    let (pool, _dir) = test_pool().await;

    let wd = WatchedDirectoryRepository::create(&pool, "/mnt/books", WatchMode::Poll, Some(30))
        .await
        .unwrap();

    // Only update watch_mode, leave others unchanged
    let updated =
        WatchedDirectoryRepository::update(&pool, wd.id, Some(WatchMode::Native), None, None)
            .await
            .unwrap();

    assert_eq!(updated.watch_mode, WatchMode::Native);
    assert_eq!(updated.poll_interval_secs, Some(30)); // unchanged
    assert!(updated.enabled); // unchanged
}

#[tokio::test]
async fn watched_directory_update_clear_poll_interval() {
    let (pool, _dir) = test_pool().await;

    let wd = WatchedDirectoryRepository::create(&pool, "/mnt/books", WatchMode::Poll, Some(30))
        .await
        .unwrap();
    assert_eq!(wd.poll_interval_secs, Some(30));

    // Set poll_interval_secs to None (use global default)
    let updated = WatchedDirectoryRepository::update(&pool, wd.id, None, Some(None), None)
        .await
        .unwrap();

    assert!(updated.poll_interval_secs.is_none());
}

#[tokio::test]
async fn watched_directory_set_last_error() {
    let (pool, _dir) = test_pool().await;

    let wd = WatchedDirectoryRepository::create(&pool, "/mnt/books", WatchMode::Poll, None)
        .await
        .unwrap();

    // Set error
    WatchedDirectoryRepository::set_last_error(&pool, wd.id, Some("permission denied"))
        .await
        .unwrap();

    let fetched = WatchedDirectoryRepository::get_by_id(&pool, wd.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.last_error.as_deref(), Some("permission denied"));

    // Clear error
    WatchedDirectoryRepository::set_last_error(&pool, wd.id, None)
        .await
        .unwrap();

    let fetched = WatchedDirectoryRepository::get_by_id(&pool, wd.id)
        .await
        .unwrap()
        .unwrap();
    assert!(fetched.last_error.is_none());
}

#[tokio::test]
async fn watched_directory_delete() {
    let (pool, _dir) = test_pool().await;

    let wd = WatchedDirectoryRepository::create(&pool, "/mnt/books", WatchMode::Poll, None)
        .await
        .unwrap();

    WatchedDirectoryRepository::delete(&pool, wd.id)
        .await
        .unwrap();

    let fetched = WatchedDirectoryRepository::get_by_id(&pool, wd.id)
        .await
        .unwrap();
    assert!(fetched.is_none());
}

#[tokio::test]
async fn watched_directory_delete_not_found() {
    let (pool, _dir) = test_pool().await;
    let result = WatchedDirectoryRepository::delete(&pool, Uuid::new_v4()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn watched_directory_exists_by_path() {
    let (pool, _dir) = test_pool().await;

    assert!(
        !WatchedDirectoryRepository::exists_by_path(&pool, "/mnt/books")
            .await
            .unwrap()
    );

    WatchedDirectoryRepository::create(&pool, "/mnt/books", WatchMode::Poll, None)
        .await
        .unwrap();

    assert!(
        WatchedDirectoryRepository::exists_by_path(&pool, "/mnt/books")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn watched_directory_get_not_found() {
    let (pool, _dir) = test_pool().await;
    let result = WatchedDirectoryRepository::get_by_id(&pool, Uuid::new_v4())
        .await
        .unwrap();
    assert!(result.is_none());
}

// ── BookFilter expanded tests ──────────────────────────────────

/// Helper to seed a book with optional properties, returning its ID.
async fn seed_book(pool: &DbPool, title: &str, f: impl FnOnce(&mut Book)) -> uuid::Uuid {
    let mut book = test_book(title);
    f(&mut book);
    BookRepository::create(pool, &book).await.unwrap();
    book.id
}

#[tokio::test]
async fn filter_tag_any_mode() {
    let (pool, _dir) = test_pool().await;
    let tag_a = Tag::new("fantasy");
    let tag_b = Tag::new("scifi");
    TagRepository::create(&pool, &tag_a).await.unwrap();
    TagRepository::create(&pool, &tag_b).await.unwrap();

    let id1 = seed_book(&pool, "Fantasy Book", |_| {}).await;
    let id2 = seed_book(&pool, "SciFi Book", |_| {}).await;
    let _id3 = seed_book(&pool, "Romance Book", |_| {}).await;

    BookRepository::add_tag(&pool, id1, tag_a.id).await.unwrap();
    BookRepository::add_tag(&pool, id2, tag_b.id).await.unwrap();

    let filter = BookFilter {
        tags: Some(vec![tag_a.id.to_string(), tag_b.id.to_string()]),
        tag_match: TagMatchMode::Any,
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 2);
}

#[tokio::test]
async fn filter_tag_all_mode() {
    let (pool, _dir) = test_pool().await;
    let tag_a = Tag::new("fantasy");
    let tag_b = Tag::new("epic");
    TagRepository::create(&pool, &tag_a).await.unwrap();
    TagRepository::create(&pool, &tag_b).await.unwrap();

    let id1 = seed_book(&pool, "Both Tags", |_| {}).await;
    let id2 = seed_book(&pool, "One Tag", |_| {}).await;

    BookRepository::add_tag(&pool, id1, tag_a.id).await.unwrap();
    BookRepository::add_tag(&pool, id1, tag_b.id).await.unwrap();
    BookRepository::add_tag(&pool, id2, tag_a.id).await.unwrap();

    let filter = BookFilter {
        tags: Some(vec![tag_a.id.to_string(), tag_b.id.to_string()]),
        tag_match: TagMatchMode::All,
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Both Tags");
}

#[tokio::test]
async fn filter_identifier_lookup() {
    let (pool, _dir) = test_pool().await;
    let id1 = seed_book(&pool, "ISBN Book", |_| {}).await;
    let _id2 = seed_book(&pool, "Other Book", |_| {}).await;

    let ident = Identifier::new(
        id1,
        IdentifierType::Isbn13,
        "9780451524935",
        MetadataSource::User,
        1.0,
    );
    IdentifierRepository::create(&pool, &ident).await.unwrap();

    let filter = BookFilter {
        identifier_types: Some(vec!["isbn13".into()]),
        identifier_value: Some("9780451524935".into()),
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "ISBN Book");
}

/// Regression: `LibraryFilterState.identifier_*` with `isbn` must match books stored as `isbn13`.
#[tokio::test]
async fn filter_isbn_via_library_filter_state_isbn13() {
    let (pool, _dir) = test_pool().await;
    let id1 = seed_book(&pool, "ISBN13 Book", |_| {}).await;
    let _id2 = seed_book(&pool, "No Ident Book", |_| {}).await;

    let ident = Identifier::new(
        id1,
        IdentifierType::Isbn13,
        "9780451524935",
        MetadataSource::User,
        1.0,
    );
    IdentifierRepository::create(&pool, &ident).await.unwrap();

    let lfs = LibraryFilterState {
        identifier_type: Some("isbn".into()),
        identifier_value: Some("9780451524935".into()),
        ..LibraryFilterState::default()
    };
    let filter = BookFilter::from(&lfs);
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "ISBN13 Book");
}

/// Regression: `LibraryFilterState.identifier_*` with `isbn` must also match books stored as `isbn10`.
#[tokio::test]
async fn filter_isbn_via_library_filter_state_isbn10() {
    let (pool, _dir) = test_pool().await;
    let id1 = seed_book(&pool, "ISBN10 Book", |_| {}).await;
    let _id2 = seed_book(&pool, "Other Book", |_| {}).await;

    let ident = Identifier::new(
        id1,
        IdentifierType::Isbn10,
        "0451524934",
        MetadataSource::User,
        1.0,
    );
    IdentifierRepository::create(&pool, &ident).await.unwrap();

    let lfs = LibraryFilterState {
        identifier_type: Some("isbn".into()),
        identifier_value: Some("0451524934".into()),
        ..LibraryFilterState::default()
    };
    let filter = BookFilter::from(&lfs);
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "ISBN10 Book");
}

/// Regression: `LibraryFilterState.identifier_*` with hyphenated ISBN input is canonicalized
/// and matches the stored value.
#[tokio::test]
async fn filter_isbn_via_library_filter_state_canonicalized() {
    let (pool, _dir) = test_pool().await;
    let id1 = seed_book(&pool, "Hyphen Book", |_| {}).await;

    let ident = Identifier::new(
        id1,
        IdentifierType::Isbn13,
        "9783161484100",
        MetadataSource::User,
        1.0,
    );
    IdentifierRepository::create(&pool, &ident).await.unwrap();

    // Input with hyphens — canonicalize strips them
    let mut lfs = LibraryFilterState {
        identifier_type: Some("isbn".into()),
        identifier_value: Some("978-3-16-148410-0".into()),
        ..LibraryFilterState::default()
    };
    lfs.canonicalize();
    assert_eq!(lfs.identifier_type.as_deref(), Some("isbn"));
    assert_eq!(lfs.identifier_value.as_deref(), Some("9783161484100"));

    let filter = BookFilter::from(&lfs);
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Hyphen Book");
}

/// ASIN lookup via `LibraryFilterState` still works (single type, not expanded).
#[tokio::test]
async fn filter_asin_via_library_filter_state() {
    let (pool, _dir) = test_pool().await;
    let id1 = seed_book(&pool, "ASIN Book", |_| {}).await;

    let ident = Identifier::new(
        id1,
        IdentifierType::Asin,
        "B08N5WRWNW",
        MetadataSource::User,
        1.0,
    );
    IdentifierRepository::create(&pool, &ident).await.unwrap();

    let lfs = LibraryFilterState {
        identifier_type: Some("asin".into()),
        identifier_value: Some("B08N5WRWNW".into()),
        ..LibraryFilterState::default()
    };
    let filter = BookFilter::from(&lfs);
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "ASIN Book");
}

#[tokio::test]
async fn filter_locked() {
    let (pool, _dir) = test_pool().await;
    seed_book(&pool, "Locked", |b| b.metadata_locked = true).await;
    seed_book(&pool, "Unlocked", |_| {}).await;

    let filter = BookFilter {
        locked: Some(true),
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Locked");
}

#[tokio::test]
async fn filter_resolution_state() {
    let (pool, _dir) = test_pool().await;
    seed_book(&pool, "Done", |b| {
        b.resolution_state = ResolutionState::Done;
    })
    .await;
    seed_book(&pool, "Pending", |_| {}).await;

    let filter = BookFilter {
        resolution_state: Some(ResolutionState::Done),
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Done");
}

#[tokio::test]
async fn filter_resolution_outcome() {
    let (pool, _dir) = test_pool().await;
    seed_book(&pool, "Confirmed", |b| {
        b.resolution_outcome = Some(ResolutionOutcome::Confirmed);
    })
    .await;
    seed_book(&pool, "No Outcome", |_| {}).await;

    let filter = BookFilter {
        resolution_outcome: Some(ResolutionOutcome::Confirmed),
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Confirmed");
}

#[tokio::test]
async fn filter_language() {
    let (pool, _dir) = test_pool().await;
    seed_book(&pool, "English", |b| b.language = Some("en".into())).await;
    seed_book(&pool, "German", |b| b.language = Some("de".into())).await;

    let filter = BookFilter {
        language: Some("en".into()),
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "English");
}

#[tokio::test]
async fn filter_year_range() {
    let (pool, _dir) = test_pool().await;
    seed_book(&pool, "Old", |b| b.publication_year = Some(1900)).await;
    seed_book(&pool, "Recent", |b| b.publication_year = Some(2020)).await;
    seed_book(&pool, "No Year", |_| {}).await;

    let filter = BookFilter {
        year_min: Some(2000),
        year_max: Some(2025),
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Recent");
}

#[tokio::test]
async fn filter_has_cover() {
    let (pool, _dir) = test_pool().await;
    seed_book(&pool, "With Cover", |b| {
        b.cover_path = Some("/covers/book.jpg".into());
    })
    .await;
    seed_book(&pool, "No Cover", |_| {}).await;

    let filter = BookFilter {
        has_cover: Some(true),
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "With Cover");
}

#[tokio::test]
async fn filter_has_description() {
    let (pool, _dir) = test_pool().await;
    seed_book(&pool, "With Desc", |b| {
        b.description = Some("A great book".into());
    })
    .await;
    seed_book(&pool, "No Desc", |_| {}).await;

    let filter = BookFilter {
        has_description: Some(true),
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "With Desc");
}

#[tokio::test]
async fn filter_has_identifiers() {
    let (pool, _dir) = test_pool().await;
    let id1 = seed_book(&pool, "Has ISBN", |_| {}).await;
    let _id2 = seed_book(&pool, "No ISBN", |_| {}).await;

    let ident = Identifier::new(
        id1,
        IdentifierType::Isbn13,
        "9780451524935",
        MetadataSource::User,
        1.0,
    );
    IdentifierRepository::create(&pool, &ident).await.unwrap();

    let filter = BookFilter {
        has_identifiers: Some(true),
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Has ISBN");
}

#[tokio::test]
async fn filter_combined() {
    let (pool, _dir) = test_pool().await;

    // Book that matches all filters
    let id1 = seed_book(&pool, "Match", |b| {
        b.language = Some("en".into());
        b.cover_path = Some("/covers/match.jpg".into());
    })
    .await;
    let file = BookFile::new(id1, BookFormat::Epub, "match.epub", 100, "hash-match", None);
    BookFileRepository::create(&pool, &file).await.unwrap();

    // Book that matches some but not all
    seed_book(&pool, "Partial", |b| {
        b.language = Some("en".into());
    })
    .await;

    let filter = BookFilter {
        format: Some(BookFormat::Epub),
        language: Some("en".into()),
        has_cover: Some(true),
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Match");
}

#[tokio::test]
async fn count_and_list_agree() {
    let (pool, _dir) = test_pool().await;
    seed_book(&pool, "A", |b| b.language = Some("en".into())).await;
    seed_book(&pool, "B", |b| b.language = Some("en".into())).await;
    seed_book(&pool, "C", |b| b.language = Some("de".into())).await;

    let filter = BookFilter {
        language: Some("en".into()),
        ..BookFilter::default()
    };

    let count = BookRepository::count(&pool, &filter).await.unwrap();
    let list = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(count, 2);
    assert_eq!(u64::from(list.total), count);
}

#[tokio::test]
async fn list_ids_matches_list() {
    let (pool, _dir) = test_pool().await;
    seed_book(&pool, "A", |b| b.language = Some("en".into())).await;
    seed_book(&pool, "B", |b| b.language = Some("en".into())).await;
    seed_book(&pool, "C", |b| b.language = Some("de".into())).await;

    let filter = BookFilter {
        language: Some("en".into()),
        ..BookFilter::default()
    };

    let ids = BookRepository::list_ids(&pool, &filter).await.unwrap();
    let list = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(ids.len(), list.items.len());
    let list_ids: Vec<_> = list.items.iter().map(|b| b.id).collect();
    for id in &ids {
        assert!(list_ids.contains(id));
    }
}

#[tokio::test]
async fn list_ids_returns_all_without_pagination() {
    let (pool, _dir) = test_pool().await;
    for i in 0..30 {
        seed_book(&pool, &format!("Book {i}"), |_| {}).await;
    }

    let ids = BookRepository::list_ids(&pool, &BookFilter::default())
        .await
        .unwrap();
    assert_eq!(ids.len(), 30);

    // Paginated list with default 25 per_page returns only 25
    let list = BookRepository::list(&pool, &PaginationParams::default(), &BookFilter::default())
        .await
        .unwrap();
    assert_eq!(list.items.len(), 25);
    assert_eq!(list.total, 30);
}

#[tokio::test]
async fn filter_trusted() {
    let (pool, _dir) = test_pool().await;
    seed_book(&pool, "Trusted", |b| b.metadata_user_trusted = true).await;
    seed_book(&pool, "Not Trusted", |_| {}).await;

    let filter = BookFilter {
        trusted: Some(true),
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Trusted");
}

#[tokio::test]
async fn filter_publisher() {
    let (pool, _dir) = test_pool().await;
    let pub1 = Publisher::new("Tor Books");
    archivis_db::PublisherRepository::create(&pool, &pub1)
        .await
        .unwrap();

    seed_book(&pool, "Tor Book", |b| b.publisher_id = Some(pub1.id)).await;
    seed_book(&pool, "Other Book", |_| {}).await;

    let filter = BookFilter {
        publisher_id: Some(pub1.id.to_string()),
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "Tor Book");
}

// ── FTS V2 trigger tests ───────────────────────────────────────

/// Helper: search for a query string and return matching titles.
async fn fts_search_titles(pool: &DbPool, query: &str) -> Vec<String> {
    let result = BookRepository::search(pool, query, &PaginationParams::default())
        .await
        .unwrap();
    result.items.into_iter().map(|b| b.title).collect()
}

#[tokio::test]
async fn fts_v2_book_with_all_relations() {
    let (pool, _dir) = test_pool().await;

    // Create related entities
    let author = test_author("Brandon Sanderson");
    AuthorRepository::create(&pool, &author).await.unwrap();
    let series = Series::new("Cosmere");
    SeriesRepository::create(&pool, &series).await.unwrap();
    let publisher = Publisher::new("Tor Books");
    PublisherRepository::create(&pool, &publisher)
        .await
        .unwrap();
    let tag = Tag::new("fantasy");
    TagRepository::create(&pool, &tag).await.unwrap();

    // Create book with publisher
    let book_id = seed_book(&pool, "The Way of Kings", |b| {
        b.publisher_id = Some(publisher.id);
    })
    .await;

    // Link relations
    BookRepository::add_author(&pool, book_id, author.id, "author", 0)
        .await
        .unwrap();
    BookRepository::add_series(&pool, book_id, series.id, Some(1.0))
        .await
        .unwrap();
    BookRepository::add_tag(&pool, book_id, tag.id)
        .await
        .unwrap();

    // Verify FTS contains all denormalized names
    assert_eq!(
        fts_search_titles(&pool, "Way Kings").await,
        vec!["The Way of Kings"]
    );
    assert_eq!(
        fts_search_titles(&pool, "Sanderson").await,
        vec!["The Way of Kings"]
    );
    assert_eq!(
        fts_search_titles(&pool, "Cosmere").await,
        vec!["The Way of Kings"]
    );
    assert_eq!(
        fts_search_titles(&pool, "Tor Books").await,
        vec!["The Way of Kings"]
    );
    assert_eq!(
        fts_search_titles(&pool, "fantasy").await,
        vec!["The Way of Kings"]
    );
}

#[tokio::test]
async fn fts_v2_update_author_name() {
    let (pool, _dir) = test_pool().await;
    let mut author = test_author("Brandos Sandersun");
    AuthorRepository::create(&pool, &author).await.unwrap();
    let book_id = seed_book(&pool, "Mistborn", |_| {}).await;
    BookRepository::add_author(&pool, book_id, author.id, "author", 0)
        .await
        .unwrap();

    // Before rename
    assert_eq!(
        fts_search_titles(&pool, "Sandersun").await,
        vec!["Mistborn"]
    );
    assert!(fts_search_titles(&pool, "Sanderson").await.is_empty());

    // Rename author
    author.name = "Brandon Sanderson".into();
    AuthorRepository::update(&pool, &author).await.unwrap();

    // After rename
    assert!(fts_search_titles(&pool, "Sandersun").await.is_empty());
    assert_eq!(
        fts_search_titles(&pool, "Sanderson").await,
        vec!["Mistborn"]
    );
}

#[tokio::test]
async fn fts_v2_update_series_name() {
    let (pool, _dir) = test_pool().await;
    let mut series = Series::new("Cosmear");
    SeriesRepository::create(&pool, &series).await.unwrap();
    let book_id = seed_book(&pool, "Elantris", |_| {}).await;
    BookRepository::add_series(&pool, book_id, series.id, None)
        .await
        .unwrap();

    assert_eq!(fts_search_titles(&pool, "Cosmear").await, vec!["Elantris"]);

    series.name = "Cosmere".into();
    SeriesRepository::update(&pool, &series).await.unwrap();

    assert!(fts_search_titles(&pool, "Cosmear").await.is_empty());
    assert_eq!(fts_search_titles(&pool, "Cosmere").await, vec!["Elantris"]);
}

#[tokio::test]
async fn fts_v2_update_tag_name() {
    let (pool, _dir) = test_pool().await;
    let mut tag = Tag::new("fantsy");
    TagRepository::create(&pool, &tag).await.unwrap();
    let book_id = seed_book(&pool, "Warbreaker", |_| {}).await;
    BookRepository::add_tag(&pool, book_id, tag.id)
        .await
        .unwrap();

    assert_eq!(fts_search_titles(&pool, "fantsy").await, vec!["Warbreaker"]);

    tag.name = "fantasy".into();
    TagRepository::update(&pool, &tag).await.unwrap();

    assert!(fts_search_titles(&pool, "fantsy").await.is_empty());
    assert_eq!(
        fts_search_titles(&pool, "fantasy").await,
        vec!["Warbreaker"]
    );
}

#[tokio::test]
async fn fts_v2_update_publisher_name() {
    let (pool, _dir) = test_pool().await;
    let mut publisher = Publisher::new("Torr");
    PublisherRepository::create(&pool, &publisher)
        .await
        .unwrap();
    seed_book(&pool, "Steelheart", |b| {
        b.publisher_id = Some(publisher.id);
    })
    .await;

    assert_eq!(fts_search_titles(&pool, "Torr").await, vec!["Steelheart"]);

    publisher.name = "Tor Books".into();
    PublisherRepository::update(&pool, &publisher)
        .await
        .unwrap();

    assert!(fts_search_titles(&pool, "Torr").await.is_empty());
    assert_eq!(
        fts_search_titles(&pool, "Tor Books").await,
        vec!["Steelheart"]
    );
}

#[tokio::test]
async fn fts_v2_add_remove_tag() {
    let (pool, _dir) = test_pool().await;
    let tag = Tag::new("epic");
    TagRepository::create(&pool, &tag).await.unwrap();
    let book_id = seed_book(&pool, "Skyward", |_| {}).await;

    // Before adding tag
    assert!(fts_search_titles(&pool, "epic").await.is_empty());

    // Add tag
    BookRepository::add_tag(&pool, book_id, tag.id)
        .await
        .unwrap();
    assert_eq!(fts_search_titles(&pool, "epic").await, vec!["Skyward"]);

    // Remove tag via `clear_tags`
    BookRepository::clear_tags(&pool, book_id).await.unwrap();
    assert!(fts_search_titles(&pool, "epic").await.is_empty());
}

#[tokio::test]
async fn fts_v2_add_remove_series() {
    let (pool, _dir) = test_pool().await;
    let series = Series::new("Skyward Flight");
    SeriesRepository::create(&pool, &series).await.unwrap();
    let book_id = seed_book(&pool, "Starsight", |_| {}).await;

    assert!(fts_search_titles(&pool, "Skyward Flight").await.is_empty());

    BookRepository::add_series(&pool, book_id, series.id, None)
        .await
        .unwrap();
    assert_eq!(
        fts_search_titles(&pool, "Skyward Flight").await,
        vec!["Starsight"]
    );

    BookRepository::clear_series(&pool, book_id).await.unwrap();
    assert!(fts_search_titles(&pool, "Skyward Flight").await.is_empty());
}

#[tokio::test]
async fn fts_v2_change_publisher() {
    let (pool, _dir) = test_pool().await;
    let pub1 = Publisher::new("Delacorte Press");
    let pub2 = Publisher::new("Random House");
    PublisherRepository::create(&pool, &pub1).await.unwrap();
    PublisherRepository::create(&pool, &pub2).await.unwrap();

    let mut book = test_book("Cytonic");
    book.publisher_id = Some(pub1.id);
    BookRepository::create(&pool, &book).await.unwrap();

    assert_eq!(fts_search_titles(&pool, "Delacorte").await, vec!["Cytonic"]);
    assert!(fts_search_titles(&pool, "Random House").await.is_empty());

    // Change publisher
    book.publisher_id = Some(pub2.id);
    BookRepository::update(&pool, &book).await.unwrap();

    assert!(fts_search_titles(&pool, "Delacorte").await.is_empty());
    assert_eq!(
        fts_search_titles(&pool, "Random House").await,
        vec!["Cytonic"]
    );
}

#[tokio::test]
async fn fts_v2_search_by_series_name() {
    let (pool, _dir) = test_pool().await;
    let series = Series::new("Stormlight Archive");
    SeriesRepository::create(&pool, &series).await.unwrap();

    let book_id = seed_book(&pool, "Rhythm of War", |_| {}).await;
    seed_book(&pool, "Unrelated Book", |_| {}).await;
    BookRepository::add_series(&pool, book_id, series.id, None)
        .await
        .unwrap();

    let titles = fts_search_titles(&pool, "Stormlight").await;
    assert_eq!(titles, vec!["Rhythm of War"]);
}

#[tokio::test]
async fn fts_v2_search_by_publisher_name() {
    let (pool, _dir) = test_pool().await;
    let publisher = Publisher::new("Dragonsteel Entertainment");
    PublisherRepository::create(&pool, &publisher)
        .await
        .unwrap();

    seed_book(&pool, "Tress of the Emerald Sea", |b| {
        b.publisher_id = Some(publisher.id);
    })
    .await;
    seed_book(&pool, "Another Book", |_| {}).await;

    let titles = fts_search_titles(&pool, "Dragonsteel").await;
    assert_eq!(titles, vec!["Tress of the Emerald Sea"]);
}

#[tokio::test]
async fn fts_v2_search_by_tag_name() {
    let (pool, _dir) = test_pool().await;
    let tag = Tag::new("progression");
    TagRepository::create(&pool, &tag).await.unwrap();

    let book_id = seed_book(&pool, "Sufficiently Advanced Magic", |_| {}).await;
    seed_book(&pool, "Some Other Book", |_| {}).await;
    BookRepository::add_tag(&pool, book_id, tag.id)
        .await
        .unwrap();

    let titles = fts_search_titles(&pool, "progression").await;
    assert_eq!(titles, vec!["Sufficiently Advanced Magic"]);
}

// ── Search sort-default tests ──────────────────────────────────

#[tokio::test]
async fn search_default_sort_is_relevance() {
    let (pool, _dir) = test_pool().await;

    // Create two books; one has the term in the title (high relevance),
    // the other only in the description (lower relevance).
    let mut b1 = test_book("Zzz Unrelated Title");
    b1.description = Some("sanderson".into());
    BookRepository::create(&pool, &b1).await.unwrap();

    let b2 = test_book("Sanderson Book");
    BookRepository::create(&pool, &b2).await.unwrap();

    // Default params → `search()` should use relevance sort, putting
    // the title match first regardless of `added_at` order.
    let result = BookRepository::search(&pool, "sanderson", &PaginationParams::default())
        .await
        .unwrap();

    assert_eq!(result.items.len(), 2);
    // Title match ranks higher than description-only match
    assert_eq!(result.items[0].title, "Sanderson Book");
}

#[tokio::test]
async fn search_explicit_sort_by_title_honored() {
    let (pool, _dir) = test_pool().await;

    seed_book(&pool, "Zebra", |b| {
        b.description = Some("sanderson".into());
    })
    .await;
    seed_book(&pool, "Alpha", |b| {
        b.description = Some("sanderson".into());
    })
    .await;

    let params = PaginationParams {
        sort_by: "title".into(),
        sort_order: SortOrder::Asc,
        ..PaginationParams::default()
    };
    let result = BookRepository::search(&pool, "sanderson", &params)
        .await
        .unwrap();

    assert_eq!(result.items.len(), 2);
    assert_eq!(result.items[0].title, "Alpha");
    assert_eq!(result.items[1].title, "Zebra");
}

#[tokio::test]
async fn list_without_query_defaults_to_added_at() {
    let (pool, _dir) = test_pool().await;

    // Seed two books; first created = earlier `added_at`
    seed_book(&pool, "First Added", |_| {}).await;
    seed_book(&pool, "Second Added", |_| {}).await;

    // Default params → `list()` should use `added_at DESC`
    let result = BookRepository::list(&pool, &PaginationParams::default(), &BookFilter::default())
        .await
        .unwrap();

    assert_eq!(result.items.len(), 2);
    // DESC: most recently added first
    assert_eq!(result.items[0].title, "Second Added");
    assert_eq!(result.items[1].title, "First Added");
}

// ── count_scope: SQL-based exact count with exclusions ──────────────

#[tokio::test]
async fn count_scope_no_exclusions_matches_count() {
    let (pool, _dir) = test_pool().await;
    for i in 0..5 {
        seed_book(&pool, &format!("Book {i}"), |_| {}).await;
    }
    let filter = BookFilter::default();
    let count = BookRepository::count(&pool, &filter).await.unwrap();
    let scoped = BookRepository::count_scope(&pool, &filter, &[])
        .await
        .unwrap();
    assert_eq!(count, scoped);
}

#[tokio::test]
async fn count_scope_subtracts_in_scope_exclusions() {
    let (pool, _dir) = test_pool().await;
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(seed_book(&pool, &format!("Book {i}"), |_| {}).await);
    }
    let filter = BookFilter::default();
    let scoped = BookRepository::count_scope(&pool, &filter, &[ids[0], ids[2]])
        .await
        .unwrap();
    assert_eq!(scoped, 3);
}

#[tokio::test]
async fn count_scope_ignores_duplicate_exclusions() {
    let (pool, _dir) = test_pool().await;
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(seed_book(&pool, &format!("Book {i}"), |_| {}).await);
    }
    let filter = BookFilter::default();
    // Same ID three times → should only subtract 1.
    let scoped = BookRepository::count_scope(&pool, &filter, &[ids[0], ids[0], ids[0]])
        .await
        .unwrap();
    assert_eq!(scoped, 4);
}

#[tokio::test]
async fn count_scope_ignores_out_of_scope_exclusions() {
    let (pool, _dir) = test_pool().await;
    for i in 0..3 {
        seed_book(&pool, &format!("Book {i}"), |_| {}).await;
    }
    let filter = BookFilter::default();
    let bogus = [Uuid::new_v4(), Uuid::new_v4()];
    let scoped = BookRepository::count_scope(&pool, &filter, &bogus)
        .await
        .unwrap();
    assert_eq!(scoped, 3, "out-of-scope IDs must not reduce count");
}

#[tokio::test]
async fn count_scope_mixed_duplicate_and_out_of_scope() {
    let (pool, _dir) = test_pool().await;
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(seed_book(&pool, &format!("Book {i}"), |_| {}).await);
    }
    let filter = BookFilter::default();
    // 1 real ID (duplicated) + 1 out-of-scope → effective exclusion = 1
    let excluded = vec![ids[1], ids[1], Uuid::new_v4()];
    let scoped = BookRepository::count_scope(&pool, &filter, &excluded)
        .await
        .unwrap();
    assert_eq!(scoped, 4);
}

#[tokio::test]
async fn count_scope_with_filter_and_exclusions() {
    let (pool, _dir) = test_pool().await;
    let mut en_ids = Vec::new();
    for i in 0..4 {
        en_ids.push(
            seed_book(&pool, &format!("EN {i}"), |b| {
                b.language = Some("en".into());
            })
            .await,
        );
    }
    // These should not be counted even without exclusion.
    for i in 0..2 {
        seed_book(&pool, &format!("DE {i}"), |b| {
            b.language = Some("de".into());
        })
        .await;
    }

    let filter = BookFilter {
        language: Some("en".into()),
        ..Default::default()
    };
    // Exclude 1 real en-book + 1 de-book (out of scope for this filter).
    let de_book = Uuid::new_v4(); // doesn't even exist
    let excluded = vec![en_ids[0], de_book];
    let scoped = BookRepository::count_scope(&pool, &filter, &excluded)
        .await
        .unwrap();
    assert_eq!(scoped, 3);
}

// ── FTS prefix search tests ───────────────────────────────────────

#[tokio::test]
async fn fts_prefix_search_partial_title() {
    let (pool, _dir) = test_pool().await;
    seed_book(&pool, "Mistborn: The Final Empire", |_| {}).await;
    seed_book(&pool, "Dune", |_| {}).await;

    let hits = fts_search_titles(&pool, "mistbo").await;
    assert_eq!(hits, vec!["Mistborn: The Final Empire"]);
}

#[tokio::test]
async fn fts_prefix_search_partial_author_name() {
    let (pool, _dir) = test_pool().await;

    let book_id = seed_book(&pool, "Wiedźmin", |_| {}).await;
    let author = test_author("Andrzej Sapkowski");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book_id, author.id, "author", 0)
        .await
        .unwrap();

    let hits = fts_search_titles(&pool, "sapkow").await;
    assert_eq!(hits, vec!["Wiedźmin"]);
}

#[tokio::test]
async fn fts_prefix_search_full_word_still_works() {
    let (pool, _dir) = test_pool().await;
    seed_book(&pool, "Dune", |_| {}).await;

    let hits = fts_search_titles(&pool, "dune").await;
    assert_eq!(hits, vec!["Dune"]);
}

#[tokio::test]
async fn fts_prefix_search_quoted_phrase_no_prefix() {
    let (pool, _dir) = test_pool().await;

    let book_id = seed_book(&pool, "Wiedźmin", |_| {}).await;
    let author = test_author("Andrzej Sapkowski");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book_id, author.id, "author", 0)
        .await
        .unwrap();

    // Quoted partial should NOT match (exact phrase, no prefix)
    let hits = fts_search_titles(&pool, r#""sapkow""#).await;
    assert!(hits.is_empty(), "quoted partial must not prefix-match");
}

#[tokio::test]
async fn fts_prefix_search_does_not_regress_relevance() {
    let (pool, _dir) = test_pool().await;

    // Title match should still rank above description-only match
    let mut b1 = test_book("Zzz Unrelated Title");
    b1.description = Some("sanderson".into());
    BookRepository::create(&pool, &b1).await.unwrap();

    let b2 = test_book("Sanderson Book");
    BookRepository::create(&pool, &b2).await.unwrap();

    let result = BookRepository::search(&pool, "sanderson", &PaginationParams::default())
        .await
        .unwrap();

    assert_eq!(result.items.len(), 2);
    assert_eq!(result.items[0].title, "Sanderson Book");
}

// ── SearchResolver: relation DSL semantics ─────────────────────

#[tokio::test]
async fn search_resolve_author_exact_match() {
    let (pool, _dir) = test_pool().await;
    let author = test_author("Stephen King");
    AuthorRepository::create(&pool, &author).await.unwrap();

    let parsed = parse_search_query("author:\"Stephen King\"");
    let resolved = SearchResolver::resolve(&pool, &parsed).await.unwrap();

    assert_eq!(resolved.author_id, Some(author.id));
    assert!(resolved.fts_column_filters.is_empty());
    assert!(resolved.warnings.is_empty());
}

#[tokio::test]
async fn search_resolve_author_single_substring() {
    let (pool, _dir) = test_pool().await;
    let author = test_author("Stephen King");
    AuthorRepository::create(&pool, &author).await.unwrap();

    let parsed = parse_search_query("author:King");
    let resolved = SearchResolver::resolve(&pool, &parsed).await.unwrap();

    assert_eq!(resolved.author_id, Some(author.id));
    assert!(resolved.fts_column_filters.is_empty());
    assert!(resolved.warnings.is_empty());
}

#[tokio::test]
async fn search_resolve_author_ambiguous_no_fts_fallback() {
    let (pool, _dir) = test_pool().await;
    let a1 = test_author("Stephen King");
    let a2 = test_author("Martin Luther King");
    AuthorRepository::create(&pool, &a1).await.unwrap();
    AuthorRepository::create(&pool, &a2).await.unwrap();

    let parsed = parse_search_query("author:King");
    let resolved = SearchResolver::resolve(&pool, &parsed).await.unwrap();

    assert!(resolved.author_id.is_none());
    assert!(resolved.fts_column_filters.is_empty());
    assert_eq!(resolved.warnings.len(), 1);
    assert!(matches!(
        &resolved.warnings[0],
        QueryWarning::AmbiguousRelation { field, match_count, .. }
        if field == "author" && *match_count == 2
    ));
}

#[tokio::test]
async fn search_resolve_author_unknown_no_fts_fallback() {
    let (pool, _dir) = test_pool().await;

    let parsed = parse_search_query("author:Nonexistent");
    let resolved = SearchResolver::resolve(&pool, &parsed).await.unwrap();

    assert!(resolved.author_id.is_none());
    assert!(resolved.fts_column_filters.is_empty());
    assert_eq!(resolved.warnings.len(), 1);
    assert!(matches!(
        &resolved.warnings[0],
        QueryWarning::UnknownRelation { field, .. } if field == "author"
    ));
}

#[tokio::test]
async fn search_resolve_author_negated_exact_preserves_fts() {
    let (pool, _dir) = test_pool().await;
    let author = test_author("Stephen King");
    AuthorRepository::create(&pool, &author).await.unwrap();

    let parsed = parse_search_query("-author:\"Stephen King\"");
    let resolved = SearchResolver::resolve(&pool, &parsed).await.unwrap();

    assert!(resolved.author_id.is_none());
    assert_eq!(resolved.fts_column_filters.len(), 1);
    assert!(resolved.fts_column_filters[0].negated);
    assert!(resolved.warnings.is_empty());
}

#[tokio::test]
async fn search_resolve_series_ambiguous_no_fts_fallback() {
    let (pool, _dir) = test_pool().await;
    let s1 = Series::new("The Dark Tower");
    let s2 = Series::new("The Dark Materials");
    SeriesRepository::create(&pool, &s1).await.unwrap();
    SeriesRepository::create(&pool, &s2).await.unwrap();

    let parsed = parse_search_query("series:Dark");
    let resolved = SearchResolver::resolve(&pool, &parsed).await.unwrap();

    assert!(resolved.series_id.is_none());
    assert!(resolved.fts_column_filters.is_empty());
    assert_eq!(resolved.warnings.len(), 1);
    assert!(matches!(
        &resolved.warnings[0],
        QueryWarning::AmbiguousRelation { field, match_count, .. }
        if field == "series" && *match_count == 2
    ));
}

#[tokio::test]
async fn search_resolve_publisher_unknown_no_fts_fallback() {
    let (pool, _dir) = test_pool().await;

    let parsed = parse_search_query("publisher:Nonexistent");
    let resolved = SearchResolver::resolve(&pool, &parsed).await.unwrap();

    assert!(resolved.publisher_id.is_none());
    assert!(resolved.fts_column_filters.is_empty());
    assert_eq!(resolved.warnings.len(), 1);
    assert!(matches!(
        &resolved.warnings[0],
        QueryWarning::UnknownRelation { field, .. } if field == "publisher"
    ));
}

#[tokio::test]
async fn search_resolve_tag_exact_ambiguous_no_fts_fallback() {
    let (pool, _dir) = test_pool().await;
    let t1 = Tag::with_category("fiction", "genre");
    let t2 = Tag::with_category("fiction", "mood");
    TagRepository::create(&pool, &t1).await.unwrap();
    TagRepository::create(&pool, &t2).await.unwrap();

    let parsed = parse_search_query("tag:fiction");
    let resolved = SearchResolver::resolve(&pool, &parsed).await.unwrap();

    assert!(resolved.tag_ids.is_empty());
    assert!(resolved.fts_column_filters.is_empty());
    assert_eq!(resolved.warnings.len(), 1);
    assert!(matches!(
        &resolved.warnings[0],
        QueryWarning::AmbiguousRelation { field, match_count, .. }
        if field == "tag" && *match_count == 2
    ));
}

#[tokio::test]
async fn search_resolve_tag_unknown_no_fts_fallback() {
    let (pool, _dir) = test_pool().await;

    let parsed = parse_search_query("tag:nonexistent");
    let resolved = SearchResolver::resolve(&pool, &parsed).await.unwrap();

    assert!(resolved.tag_ids.is_empty());
    assert!(resolved.fts_column_filters.is_empty());
    assert_eq!(resolved.warnings.len(), 1);
    assert!(matches!(
        &resolved.warnings[0],
        QueryWarning::UnknownRelation { field, .. } if field == "tag"
    ));
}
