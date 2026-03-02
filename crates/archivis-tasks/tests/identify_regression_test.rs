//! Regression tests for the identify → match → apply pipeline.
//!
//! These tests document invariants for the identification system:
//!   1) No auto-apply without exact trusted ID proof.
//!   2) ASIN must be a first-class lookup input.
//!   3) Stale author must be correctable under strong ID proof.
//!
//! At least one of these tests is expected to FAIL on the current code,
//! proving the regression exists and guiding the fix.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use archivis_core::models::{
    Book, IdentificationCandidate, Identifier, IdentifierType, MetadataSource, MetadataStatus,
};
use archivis_core::settings::SettingsReader;
use archivis_db::{AuthorRepository, BookRepository, CandidateRepository, IdentifierRepository};
use archivis_metadata::{
    MetadataProvider, MetadataQuery, MetadataResolver, ProviderAuthor, ProviderIdentifier,
    ProviderMetadata, ProviderRegistry,
};
use archivis_storage::local::LocalStorage;
use archivis_tasks::identify::IdentificationService;
use tempfile::TempDir;

// ── Test infrastructure ─────────────────────────────────────────────

/// In-memory stub for `SettingsReader`.
struct StubSettings(HashMap<String, serde_json::Value>);

impl StubSettings {
    fn new(entries: Vec<(&str, serde_json::Value)>) -> Self {
        Self(
            entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }
}

impl SettingsReader for StubSettings {
    fn get_setting(&self, key: &str) -> Option<serde_json::Value> {
        self.0.get(key).cloned()
    }
}

/// A configurable stub provider that returns different results for
/// ISBN lookup vs title+author search.
struct FlexibleStubProvider {
    isbn_results: Vec<ProviderMetadata>,
    search_results: Vec<ProviderMetadata>,
}

#[async_trait::async_trait]
impl MetadataProvider for FlexibleStubProvider {
    fn name(&self) -> &'static str {
        "test_provider"
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn lookup_isbn(
        &self,
        _isbn: &str,
    ) -> Result<Vec<ProviderMetadata>, archivis_metadata::ProviderError> {
        Ok(self.isbn_results.clone())
    }

    async fn search(
        &self,
        _query: &MetadataQuery,
    ) -> Result<Vec<ProviderMetadata>, archivis_metadata::ProviderError> {
        Ok(self.search_results.clone())
    }

    async fn fetch_cover(
        &self,
        _cover_url: &str,
    ) -> Result<Vec<u8>, archivis_metadata::ProviderError> {
        Ok(vec![])
    }
}

/// Set up a test environment with a custom provider.
async fn setup_with_provider(
    tmp: &TempDir,
    provider: Arc<dyn MetadataProvider>,
) -> (IdentificationService<LocalStorage>, archivis_db::DbPool) {
    let db_path = tmp.path().join("test.db");
    let pool = archivis_db::create_pool(&db_path).await.unwrap();
    archivis_db::run_migrations(&pool).await.unwrap();

    let storage_dir = tmp.path().join("storage");
    let storage = LocalStorage::new(&storage_dir).await.unwrap();
    let data_dir = tmp.path().join("data");

    let settings = Arc::new(StubSettings::new(vec![(
        "metadata.auto_identify_threshold",
        serde_json::json!(0.85),
    )]));

    let mut registry = ProviderRegistry::new();
    registry.register(provider);

    let resolver = Arc::new(MetadataResolver::new(Arc::new(registry), settings));
    let service = IdentificationService::new(pool.clone(), resolver, storage, data_dir);

    (service, pool)
}

/// Set up a test environment without a provider (for apply-only tests).
async fn setup_no_provider(
    tmp: &TempDir,
) -> (IdentificationService<LocalStorage>, archivis_db::DbPool) {
    let db_path = tmp.path().join("test.db");
    let pool = archivis_db::create_pool(&db_path).await.unwrap();
    archivis_db::run_migrations(&pool).await.unwrap();

    let storage_dir = tmp.path().join("storage");
    let storage = LocalStorage::new(&storage_dir).await.unwrap();
    let data_dir = tmp.path().join("data");

    let settings = Arc::new(StubSettings::new(vec![]));
    let registry = ProviderRegistry::new();
    let resolver = Arc::new(MetadataResolver::new(Arc::new(registry), settings));
    let service = IdentificationService::new(pool.clone(), resolver, storage, data_dir);

    (service, pool)
}

// ── Regression tests ────────────────────────────────────────────────

/// Regression: ASIN-only book must NOT have a wrong title auto-applied
/// via fuzzy search results.
///
/// Scenario (B00BDQ399Y regression):
///   - Book: "Mortal Arts (A Lady Darby Mystery)" with ASIN, no ISBN.
///   - Provider search returns a candidate with matching title+author
///     but NO ISBN in the result.
///   - Score exceeds 0.85 due to title+author fuzzy bonuses.
///   - Current code: auto-applies (`TitleMatch` + `AuthorMatch` passes
///     `has_multi_signal`).
///   - Expected: `NeedsReview` — without ISBN or cross-provider proof,
///     fuzzy signals alone must not trigger auto-apply.
#[tokio::test]
async fn asin_book_not_auto_applied_via_fuzzy_search() {
    let tmp = TempDir::new().unwrap();

    // Provider search returns a plausible candidate with matching
    // title + author but NO ISBN (no hard identifier proof).
    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![], // Not reached (no ISBN in query)
        search_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Mortal Arts".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Anna Lee Huber".into(),
                role: None,
            }],
            description: Some("A Lady Darby Mystery novel.".into()),
            language: None,
            publisher: None,
            publication_date: None,
            identifiers: vec![], // No ISBN!
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            confidence: 0.75,
        }],
    });

    let (service, pool) = setup_with_provider(&tmp, provider).await;

    // Create book with ASIN only (no ISBN).
    let book = Book::new("Mortal Arts (A Lady Darby Mystery)");
    BookRepository::create(&pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("Anna Lee Huber");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let asin = Identifier::new(
        book.id,
        IdentifierType::Asin,
        "B00BDQ399Y",
        MetadataSource::Embedded,
        0.9,
    );
    IdentifierRepository::create(&pool, &asin).await.unwrap();

    // Run identification.
    let outcome = service.identify_book(book.id).await.unwrap();

    assert!(
        !outcome.resolver_result.candidates.is_empty(),
        "precondition: should have at least one candidate from search"
    );

    // The best candidate should have a high score from fuzzy matching.
    let best = outcome.resolver_result.best_match.as_ref().unwrap();
    assert!(
        best.score >= 0.85,
        "precondition: score should be high from fuzzy match, got {}",
        best.score
    );

    // INVARIANT: Without ISBN match or cross-provider corroboration,
    // the book must remain NeedsReview, not auto-applied.
    let updated_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(
        updated_book.metadata_status,
        MetadataStatus::NeedsReview,
        "book should be NeedsReview (not auto-applied) without ISBN proof; \
         auto_applied={}, best_score={}, title={:?}",
        outcome.auto_applied,
        best.score,
        updated_book.title
    );
}

/// Regression: single fuzzy-only candidate must remain `NeedsReview`.
///
/// Even when a single provider returns a single candidate with good
/// title match, the absence of hard ID proof should prevent auto-apply.
/// This is the minimal reproduction of the permissive auto-apply bug.
#[tokio::test]
async fn fuzzy_only_single_candidate_remains_needs_review() {
    let tmp = TempDir::new().unwrap();

    // Provider returns a single result via search, no ISBN involved.
    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![],
        search_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Red Storm Rising".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Tom Clancy".into(),
                role: None,
            }],
            description: None,
            language: None,
            publisher: None,
            publication_date: None,
            identifiers: vec![], // No ISBN
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            confidence: 0.75,
        }],
    });

    let (service, pool) = setup_with_provider(&tmp, provider).await;

    let book = Book::new("Red Storm Rising");
    BookRepository::create(&pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("Tom Clancy");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let outcome = service.identify_book(book.id).await.unwrap();

    assert!(
        !outcome.resolver_result.candidates.is_empty(),
        "precondition: should have at least one candidate"
    );

    let updated_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(
        updated_book.metadata_status,
        MetadataStatus::NeedsReview,
        "fuzzy-only single candidate should remain NeedsReview, not auto-applied"
    );
}

/// Positive control: strong ISBN match with title+author corroboration
/// correctly triggers auto-apply.
///
/// When a candidate has:
///   - ISBN exact match (query ISBN matches candidate ISBN)
///   - Title similarity (`TitleMatch` signal)
///   - Author match (`AuthorMatch` signal)
///   - Score above threshold
/// auto-apply IS appropriate because there's hard ID proof.
#[tokio::test]
async fn strong_isbn_match_auto_applies_correctly() {
    let tmp = TempDir::new().unwrap();

    // Provider returns a result via ISBN lookup with matching identifiers.
    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Dune".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Frank Herbert".into(),
                role: None,
            }],
            description: Some("A sci-fi classic.".into()),
            language: None,
            publisher: None,
            publication_date: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            confidence: 0.95,
        }],
        search_results: vec![],
    });

    let (service, pool) = setup_with_provider(&tmp, provider).await;

    let book = Book::new("Dune");
    BookRepository::create(&pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("Frank Herbert");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let isbn = Identifier::new(
        book.id,
        IdentifierType::Isbn13,
        "9780441172719",
        MetadataSource::Embedded,
        0.9,
    );
    IdentifierRepository::create(&pool, &isbn).await.unwrap();

    let outcome = service.identify_book(book.id).await.unwrap();

    assert!(
        !outcome.resolver_result.candidates.is_empty(),
        "should have at least one candidate"
    );

    let updated_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(
        updated_book.metadata_status,
        MetadataStatus::Identified,
        "strong ISBN match should auto-apply (status should be Identified)"
    );
    assert!(
        updated_book.metadata_confidence > 0.85,
        "confidence should be above threshold after auto-apply"
    );
}

/// Regression: stale author must be correctable under strong ISBN proof.
///
/// When a book has a wrong author (from a previous bad identification),
/// re-identifying with a provider that returns the correct author via
/// strong ISBN match should update the author.
///
/// Current code only replaces authors when the existing author is
/// literally "Unknown Author". A stale wrong author persists even when
/// the new candidate has strong ISBN proof.
///
/// Invariant: "Core identity fields require stronger proof than
/// enrichment fields" — but when that proof IS present, stale data
/// must be correctable.
#[tokio::test]
async fn stale_author_replaced_under_strong_isbn_proof() {
    let tmp = TempDir::new().unwrap();
    let (service, pool) = setup_no_provider(&tmp).await;

    // Create a book with a WRONG author (from previous bad identify).
    let book = Book::new("Mortal Arts");
    BookRepository::create(&pool, &book).await.unwrap();

    // "John Bunyan" is the wrong author — this came from a previous
    // misidentification that confused "Mortal Arts" with "Pilgrim's Progress".
    let wrong_author = archivis_core::models::Author::new("John Bunyan");
    AuthorRepository::create(&pool, &wrong_author)
        .await
        .unwrap();
    BookRepository::add_author(&pool, book.id, wrong_author.id, "author", 0)
        .await
        .unwrap();

    // Book has a strong ISBN identifier.
    let isbn = Identifier::new(
        book.id,
        IdentifierType::Isbn13,
        "9780425253465",
        MetadataSource::Embedded,
        0.9,
    );
    IdentifierRepository::create(&pool, &isbn).await.unwrap();

    // Create a candidate with the CORRECT author via ISBN match.
    let correct_meta = ProviderMetadata {
        provider_name: "test_provider".into(),
        title: Some("Mortal Arts".into()),
        subtitle: None,
        authors: vec![ProviderAuthor {
            name: "Anna Lee Huber".into(),
            role: Some("author".into()),
        }],
        description: None,
        language: None,
        publisher: None,
        publication_date: None,
        identifiers: vec![ProviderIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: "9780425253465".into(),
        }],
        subjects: vec![],
        series: None,
        page_count: None,
        cover_url: None,
        rating: None,
        confidence: 0.95,
    };
    let meta_json = serde_json::to_value(&correct_meta).unwrap();
    let candidate = IdentificationCandidate::new(
        book.id,
        "test_provider",
        0.95,
        meta_json,
        vec![
            "ISBN exact match".to_string(),
            "Title fuzzy match (100%)".to_string(),
        ],
    );
    CandidateRepository::create(&pool, &candidate)
        .await
        .unwrap();

    // Apply the candidate (simulates user confirming the correct match,
    // or auto-apply with strong ISBN proof).
    let updated_book = service
        .apply_candidate(book.id, candidate.id, &HashSet::new())
        .await
        .unwrap();

    // Verify the title was updated.
    assert_eq!(updated_book.title, "Mortal Arts");

    // INVARIANT: Under strong ISBN proof, the stale wrong author
    // ("John Bunyan") must be replaced with the correct author.
    let relations = BookRepository::get_with_relations(&pool, book.id)
        .await
        .unwrap();
    let author_names: Vec<&str> = relations
        .authors
        .iter()
        .map(|a| a.author.name.as_str())
        .collect();

    assert!(
        author_names.contains(&"Anna Lee Huber"),
        "stale author should be replaced with correct author under strong ISBN proof; \
         current authors: {author_names:?}"
    );
    assert!(
        !author_names.contains(&"John Bunyan"),
        "wrong author 'John Bunyan' should be replaced; current authors: {author_names:?}"
    );
}
