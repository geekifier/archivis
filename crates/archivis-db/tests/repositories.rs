use archivis_core::models::{
    Author, Book, BookFile, BookFormat, Identifier, IdentifierType, MetadataSource, MetadataStatus,
    Publisher, Series, Tag,
};
use archivis_db::{
    create_pool, run_migrations, AuthorRepository, BookFileRepository, BookFilter, BookRepository,
    DbPool, IdentifierRepository, PaginationParams, SeriesRepository, SortOrder, TagRepository,
};
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
    book.metadata_confidence = 0.95;

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

    let file = BookFile::new(book.id, BookFormat::Epub, "path.epub", 100, "hash1");
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
    assert_eq!(result.items[0].name, "Asimov");
    assert_eq!(result.items[1].name, "Herbert");
    assert_eq!(result.items[2].name, "Tolkien");
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

    let file1 = BookFile::new(book.id, BookFormat::Epub, "path1.epub", 100, "samehash");
    BookFileRepository::create(&pool, &file1).await.unwrap();

    let file2 = BookFile::new(book.id, BookFormat::Pdf, "path2.pdf", 200, "samehash");
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
    assert_eq!(result.items[0].name, "fantasy");
    assert_eq!(result.items[1].name, "horror");
    assert_eq!(result.items[2].name, "science fiction");
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
    let file1 = BookFile::new(book1.id, BookFormat::Epub, "a.epub", 100, "h1");
    BookFileRepository::create(&pool, &file1).await.unwrap();

    let book2 = test_book("PDF Book");
    BookRepository::create(&pool, &book2).await.unwrap();
    let file2 = BookFile::new(book2.id, BookFormat::Pdf, "b.pdf", 200, "h2");
    BookFileRepository::create(&pool, &file2).await.unwrap();

    let filter = BookFilter {
        format: Some(BookFormat::Epub),
        ..BookFilter::default()
    };
    let result = BookRepository::list(&pool, &PaginationParams::default(), &filter)
        .await
        .unwrap();

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].title, "EPUB Book");
}
