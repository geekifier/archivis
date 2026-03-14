//! Regression tests for the resolve → match → apply pipeline.
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
    Book, CandidateStatus, FieldProvenance, IdentificationCandidate, Identifier, IdentifierType,
    MetadataSource, MetadataStatus, ResolutionOutcome as BookResolutionOutcome,
};
use archivis_core::settings::SettingsReader;
use archivis_db::{
    AuthorRepository, BookRepository, CandidateRepository, IdentifierRepository,
    ResolutionRunRepository,
};
use archivis_metadata::{
    MetadataProvider, MetadataQuery, MetadataResolver, ProviderAuthor, ProviderIdentifier,
    ProviderMetadata, ProviderRegistry,
};
use archivis_storage::local::LocalStorage;
use archivis_tasks::resolve::ResolutionService;
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

static FLEX_STUB_CAPS: archivis_metadata::ProviderCapabilities =
    archivis_metadata::ProviderCapabilities {
        quality: archivis_metadata::ProviderQuality::Community,
        default_rate_limit_rpm: 100,
        supported_id_lookups: &[
            IdentifierType::Isbn13,
            IdentifierType::Isbn10,
            IdentifierType::Asin,
        ],
        features: &[
            archivis_metadata::ProviderFeature::Search,
            archivis_metadata::ProviderFeature::Covers,
        ],
    };

#[async_trait::async_trait]
impl MetadataProvider for FlexibleStubProvider {
    fn name(&self) -> &'static str {
        "test_provider"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn capabilities(&self) -> &'static archivis_metadata::ProviderCapabilities {
        &FLEX_STUB_CAPS
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
) -> (ResolutionService<LocalStorage>, archivis_db::DbPool) {
    let db_path = tmp.path().join("test.db");
    let pool = archivis_db::create_pool(&db_path).await.unwrap();
    archivis_db::run_migrations(&pool).await.unwrap();

    let storage_dir = tmp.path().join("storage");
    let storage = LocalStorage::new(&storage_dir).await.unwrap();
    let data_dir = tmp.path().join("data");

    let settings = Arc::new(StubSettings::new(vec![(
        "metadata.auto_apply_threshold",
        serde_json::json!(0.85),
    )]));

    let mut registry = ProviderRegistry::new();
    registry.register(provider);

    let resolver = Arc::new(MetadataResolver::new(Arc::new(registry), settings));
    let service = ResolutionService::new(pool.clone(), resolver, storage, data_dir);

    (service, pool)
}

/// Set up a test environment without a provider (for apply-only tests).
async fn setup_no_provider(
    tmp: &TempDir,
) -> (ResolutionService<LocalStorage>, archivis_db::DbPool) {
    let db_path = tmp.path().join("test.db");
    let pool = archivis_db::create_pool(&db_path).await.unwrap();
    archivis_db::run_migrations(&pool).await.unwrap();

    let storage_dir = tmp.path().join("storage");
    let storage = LocalStorage::new(&storage_dir).await.unwrap();
    let data_dir = tmp.path().join("data");

    let settings = Arc::new(StubSettings::new(vec![]));
    let registry = ProviderRegistry::new();
    let resolver = Arc::new(MetadataResolver::new(Arc::new(registry), settings));
    let service = ResolutionService::new(pool.clone(), resolver, storage, data_dir);

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
            publication_year: None,
            identifiers: vec![], // No ISBN!
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
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
    let outcome = service.resolve_book(book.id, false).await.unwrap();

    assert!(
        !outcome.resolver_result.candidates.is_empty(),
        "precondition: should have at least one candidate from search"
    );

    // The best candidate should have a reasonable score from fuzzy matching
    // (reduced by WeakMatch tier factor since there's no ISBN proof).
    let best = outcome.resolver_result.best_match.as_ref().unwrap();
    assert!(
        best.score >= 0.50,
        "precondition: score should be reasonable from fuzzy match, got {}",
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
            publication_year: None,
            identifiers: vec![], // No ISBN
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
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

    let outcome = service.resolve_book(book.id, false).await.unwrap();

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
            publication_year: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
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

    let outcome = service.resolve_book(book.id, false).await.unwrap();

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
    // `ingest_quality_score` is import-time quality only; auto-apply must NOT overwrite it
    assert!(
        (updated_book.ingest_quality_score - book.ingest_quality_score).abs() < f32::EPSILON,
        "ingest_quality_score must stay at import-time value after auto-apply"
    );
}

/// Regression: stale author must be correctable under strong ISBN proof.
///
/// When a book has a wrong author (from a previous bad identification),
/// re-resolving with a provider that returns the correct author via
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

    // Create a book with a WRONG author (from a previous bad resolution).
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
        publication_year: None,
        identifiers: vec![ProviderIdentifier {
            identifier_type: IdentifierType::Isbn13,
            value: "9780425253465".into(),
        }],
        subjects: vec![],
        series: None,
        page_count: None,
        cover_url: None,
        rating: None,
        physical_format: None,
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

#[tokio::test]
async fn strong_match_enriches_existing_book() {
    let tmp = TempDir::new().unwrap();

    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Dune".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Frank Herbert".into(),
                role: Some("author".into()),
            }],
            description: Some("Arrakis awaits.".into()),
            language: Some("en".into()),
            publisher: Some("Ace".into()),
            publication_year: Some(1965),
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: Some(412),
            cover_url: None,
            rating: None,
            physical_format: None,
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

    let outcome = service.resolve_book(book.id, false).await.unwrap();

    assert!(
        outcome.auto_applied,
        "strong ISBN candidate should reconcile automatically"
    );

    let updated = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(updated.description.as_deref(), Some("Arrakis awaits."));
    assert_eq!(updated.language.as_deref(), Some("en"));
    assert_eq!(updated.page_count, Some(412));
    assert_eq!(
        updated.resolution_outcome,
        Some(BookResolutionOutcome::Enriched)
    );
    assert_eq!(
        updated
            .metadata_provenance
            .description
            .as_ref()
            .unwrap()
            .origin,
        MetadataSource::Provider("test_provider".into())
    );
    assert!(
        !updated
            .metadata_provenance
            .description
            .as_ref()
            .unwrap()
            .protected
    );
}

#[tokio::test]
async fn protected_core_conflict_becomes_disputed() {
    let tmp = TempDir::new().unwrap();

    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Different Title".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Frank Herbert".into(),
                role: Some("author".into()),
            }],
            description: Some("Arrakis awaits.".into()),
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.95,
        }],
        search_results: vec![],
    });

    let (service, pool) = setup_with_provider(&tmp, provider).await;

    let mut book = Book::new("Dune");
    book.metadata_provenance.title = Some(FieldProvenance {
        origin: MetadataSource::User,
        protected: true,
    });
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

    let outcome = service.resolve_book(book.id, false).await.unwrap();

    assert!(
        !outcome.auto_applied,
        "protected core conflict should stay in review"
    );

    let updated = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(updated.title, "Dune");
    assert_eq!(
        updated.resolution_outcome,
        Some(BookResolutionOutcome::Disputed)
    );

    let candidates = CandidateRepository::list_by_book(&pool, book.id)
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].status,
        archivis_core::models::CandidateStatus::Pending
    );
}

/// Regression: "The Lady of the Lake" with correct embedded metadata
/// should NOT be flagged as `Disputed` / `NeedsReview` when the provider
/// strips the leading article from the title and adds a translator as a
/// co-author.
///
/// Expected: enrichments applied (publisher), title preserved,
/// author preserved, translator NOT counted as differing author,
/// status = `Identified`.
#[tokio::test]
async fn lady_of_the_lake_article_and_translator_regression() {
    let tmp = TempDir::new().unwrap();

    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Lady of the Lake".into()),
            subtitle: None,
            authors: vec![
                ProviderAuthor {
                    name: "Andrzej Sapkowski".into(),
                    role: Some("author".into()),
                },
                // Real OL API lists translator directly in `authors` with role "author"
                ProviderAuthor {
                    name: "David A. French".into(),
                    role: Some("author".into()),
                },
            ],
            description: None,
            language: None,
            publisher: Some("Orbit".into()),
            publication_year: Some(2017),
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780316273770".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.95,
        }],
        search_results: vec![],
    });

    let (service, pool) = setup_with_provider(&tmp, provider).await;

    let book = Book::new("The Lady of the Lake");
    BookRepository::create(&pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("Andrzej Sapkowski");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let isbn = Identifier::new(
        book.id,
        IdentifierType::Isbn13,
        "9780316273770",
        MetadataSource::Embedded,
        0.9,
    );
    IdentifierRepository::create(&pool, &isbn).await.unwrap();

    let outcome = service.resolve_book(book.id, false).await.unwrap();

    assert!(
        outcome.auto_applied,
        "should auto-apply: strong ISBN match with equivalent title and author"
    );

    let updated = BookRepository::get_by_id(&pool, book.id).await.unwrap();

    // Title preserved (not overwritten with article-stripped version).
    assert_eq!(
        updated.title, "The Lady of the Lake",
        "title should be preserved, not replaced with article-stripped variant"
    );

    // Enrichments applied.
    assert!(
        updated.publication_year.is_some(),
        "publication_year should be enriched"
    );

    // Status should be Identified, not NeedsReview.
    assert_eq!(
        updated.metadata_status,
        MetadataStatus::Identified,
        "book should be Identified, not NeedsReview"
    );

    // Resolution outcome should be Enriched or Confirmed.
    assert!(
        matches!(
            updated.resolution_outcome,
            Some(BookResolutionOutcome::Enriched | BookResolutionOutcome::Confirmed)
        ),
        "expected Enriched or Confirmed, got {:?}",
        updated.resolution_outcome
    );
}

// ── Baseline resolution_outcome regression tests ────────────────

/// Regression: reject-all must restore the pre-review `resolution_outcome`.
///
/// Scenario: book is `Confirmed` after a successful identification.
/// A second `resolve_book()` produces new candidates and enters review,
/// overwriting `resolution_outcome` to `Ambiguous`. Rejecting all
/// candidates must restore `Confirmed`.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn reject_all_restores_confirmed_outcome() {
    let tmp = TempDir::new().unwrap();

    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Dune".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Frank Herbert".into(),
                role: Some("author".into()),
            }],
            description: Some("Arrakis awaits.".into()),
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780441172719".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.95,
        }],
        search_results: vec![],
    });

    let (service, pool) = setup_with_provider(&tmp, provider).await;

    // 1. Create book with ISBN, resolve → auto-apply (strong ISBN match)
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

    let outcome1 = service.resolve_book(book.id, false).await.unwrap();
    assert!(outcome1.auto_applied, "precondition: should auto-apply");

    // 2. Trust metadata to set Confirmed outcome
    let trusted = service.trust_metadata(book.id).await.unwrap();
    assert!(trusted, "trust should succeed when not Running");
    let confirmed_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(
        confirmed_book.resolution_outcome,
        Some(BookResolutionOutcome::Confirmed),
        "precondition: should be Confirmed after trust_metadata"
    );

    // 3. Second resolve → enters review (manual refresh)
    let outcome2 = service.resolve_book(book.id, true).await.unwrap();
    assert!(
        !outcome2.auto_applied,
        "manual refresh should not auto-apply"
    );

    let review_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert!(
        review_book.review_baseline_metadata_status.is_some(),
        "should have review baseline set"
    );
    assert_eq!(
        review_book.review_baseline_resolution_outcome,
        Some(BookResolutionOutcome::Confirmed),
        "baseline should capture the Confirmed outcome"
    );

    // 4. Reject all pending candidates
    let candidates = CandidateRepository::list_by_book(&pool, book.id)
        .await
        .unwrap();
    let pending_ids: Vec<_> = candidates
        .iter()
        .filter(|c| c.status == CandidateStatus::Pending)
        .map(|c| c.id)
        .collect();
    assert!(
        !pending_ids.is_empty(),
        "precondition: should have pending candidates"
    );

    service
        .reject_candidates(book.id, &pending_ids)
        .await
        .unwrap();

    // 5. Verify: outcome restored to Confirmed, baselines cleared
    let final_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(
        final_book.resolution_outcome,
        Some(BookResolutionOutcome::Confirmed),
        "resolution_outcome should be restored to Confirmed after reject-all; got {:?}",
        final_book.resolution_outcome
    );
    assert!(
        final_book.review_baseline_metadata_status.is_none(),
        "review_baseline_metadata_status should be cleared"
    );
    assert!(
        final_book.review_baseline_resolution_outcome.is_none(),
        "review_baseline_resolution_outcome should be cleared"
    );
    assert_eq!(
        final_book.metadata_status,
        MetadataStatus::Identified,
        "metadata_status should be restored to baseline Identified; got {:?}",
        final_book.metadata_status
    );
}

/// Reject-all on a fresh book (no prior outcome) restores `None` outcome.
#[tokio::test]
async fn reject_all_restores_none_outcome_for_fresh_book() {
    let tmp = TempDir::new().unwrap();

    // Provider returns a search-only result (no ISBN in result → WeakMatch tier)
    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![],
        search_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Fresh Book Title".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Some Author".into(),
                role: None,
            }],
            description: None,
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.7,
        }],
    });

    let (service, pool) = setup_with_provider(&tmp, provider).await;

    let book = Book::new("Fresh Book Title");
    assert!(
        book.resolution_outcome.is_none(),
        "precondition: fresh book has no outcome"
    );
    BookRepository::create(&pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("Some Author");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    // Resolve → candidates stored, no auto-apply (search-only, weak match)
    let outcome = service.resolve_book(book.id, false).await.unwrap();
    assert!(
        !outcome.auto_applied,
        "precondition: should not auto-apply weak match"
    );

    let review_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    let pre_review_status = review_book
        .review_baseline_metadata_status
        .expect("precondition: baseline status should be set");
    let pre_review_outcome = review_book.review_baseline_resolution_outcome;

    // Reject all pending candidates
    let candidates = CandidateRepository::list_by_book(&pool, book.id)
        .await
        .unwrap();
    let pending_ids: Vec<_> = candidates
        .iter()
        .filter(|c| c.status == CandidateStatus::Pending)
        .map(|c| c.id)
        .collect();

    if !pending_ids.is_empty() {
        service
            .reject_candidates(book.id, &pending_ids)
            .await
            .unwrap();
    }

    let final_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(
        final_book.metadata_status, pre_review_status,
        "metadata_status should be restored to pre-review value ({:?}); got {:?}",
        pre_review_status, final_book.metadata_status
    );
    assert_eq!(
        final_book.resolution_outcome, pre_review_outcome,
        "outcome should be restored to pre-review value ({:?}); got {:?}",
        pre_review_outcome, final_book.resolution_outcome
    );
    assert!(
        final_book.review_baseline_metadata_status.is_none(),
        "baseline should be cleared"
    );
    assert!(
        final_book.review_baseline_resolution_outcome.is_none(),
        "baseline outcome should be cleared"
    );
}

/// Apply then undo must restore `review_baseline_resolution_outcome` from changeset.
#[tokio::test]
async fn apply_then_undo_restores_baseline_resolution_outcome() {
    let tmp = TempDir::new().unwrap();

    // Provider returns a search-only candidate (no ISBN → won't auto-apply)
    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![],
        search_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Undo Test Book".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Test Author".into(),
                role: None,
            }],
            description: Some("A description.".into()),
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.7,
        }],
    });

    let (service, pool) = setup_with_provider(&tmp, provider).await;

    let book = Book::new("Undo Test Book");
    BookRepository::create(&pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("Test Author");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    // Resolve → candidates stored, enters review
    let outcome = service.resolve_book(book.id, false).await.unwrap();
    assert!(!outcome.auto_applied, "precondition: should not auto-apply");

    let review_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert!(
        review_book.review_baseline_metadata_status.is_some(),
        "precondition: baseline should be set"
    );

    // Find the pending candidate
    let candidates = CandidateRepository::list_by_book(&pool, book.id)
        .await
        .unwrap();
    let pending = candidates
        .iter()
        .find(|c| c.status == CandidateStatus::Pending)
        .expect("should have a pending candidate");

    // Apply → baseline cleared, outcome set
    let applied_book = service
        .apply_candidate(book.id, pending.id, &HashSet::new())
        .await
        .unwrap();
    assert!(
        applied_book.review_baseline_metadata_status.is_none(),
        "baseline should be cleared after apply"
    );
    assert!(
        applied_book.review_baseline_resolution_outcome.is_none(),
        "baseline outcome should be cleared after apply"
    );

    // Undo → baseline restored from changeset
    let undone_book = service.undo_candidate(book.id, pending.id).await.unwrap();
    assert_eq!(
        undone_book.review_baseline_metadata_status, review_book.review_baseline_metadata_status,
        "baseline status should be restored after undo"
    );
    assert_eq!(
        undone_book.review_baseline_resolution_outcome,
        review_book.review_baseline_resolution_outcome,
        "baseline outcome should be restored after undo"
    );
}

// ── Trust/untrust roundtrip regression tests ─────────────────────

/// Helper: resolve a book with a strong ISBN match, returning the auto-applied
/// outcome for use in trust/untrust tests.
async fn resolve_to_outcome(
    service: &ResolutionService<LocalStorage>,
    pool: &archivis_db::DbPool,
    title: &str,
    isbn: &str,
    provider_meta: ProviderMetadata,
) -> (uuid::Uuid, BookResolutionOutcome) {
    let book = Book::new(title);
    BookRepository::create(pool, &book).await.unwrap();

    let author_name = provider_meta
        .authors
        .first()
        .map_or("Test Author", |a| a.name.as_str());
    let author = archivis_core::models::Author::new(author_name);
    AuthorRepository::create(pool, &author).await.unwrap();
    BookRepository::add_author(pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let id = Identifier::new(
        book.id,
        IdentifierType::Isbn13,
        isbn,
        MetadataSource::Embedded,
        0.9,
    );
    IdentifierRepository::create(pool, &id).await.unwrap();

    let outcome = service.resolve_book(book.id, false).await.unwrap();
    assert!(
        outcome.auto_applied,
        "precondition: should auto-apply strong ISBN match"
    );

    let updated = BookRepository::get_by_id(pool, book.id).await.unwrap();
    let resolution_outcome = updated
        .resolution_outcome
        .expect("precondition: should have resolution outcome after auto-apply");
    (book.id, resolution_outcome)
}

/// Untrust re-evaluates: book with applied candidate recomputes to
/// `Identified` with `resolution_outcome = None`.
#[tokio::test]
async fn trust_untrust_recomputes_from_applied_candidate() {
    let tmp = TempDir::new().unwrap();

    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Enriched Book".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Author A".into(),
                role: Some("author".into()),
            }],
            description: Some("New description.".into()),
            language: Some("en".into()),
            publisher: None,
            publication_year: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780000000001".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.95,
        }],
        search_results: vec![],
    });

    let (service, pool) = setup_with_provider(&tmp, provider).await;
    let (book_id, pre_trust_outcome) = resolve_to_outcome(
        &service,
        &pool,
        "Enriched Book",
        "9780000000001",
        ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Enriched Book".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Author A".into(),
                role: Some("author".into()),
            }],
            description: Some("New description.".into()),
            language: Some("en".into()),
            publisher: None,
            publication_year: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780000000001".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.95,
        },
    )
    .await;
    assert_eq!(
        pre_trust_outcome,
        BookResolutionOutcome::Enriched,
        "precondition: auto-apply with new fields should produce Enriched"
    );

    // Trust
    let trusted = service.trust_metadata(book_id).await.unwrap();
    assert!(trusted, "trust should succeed");
    let trusted_book = BookRepository::get_by_id(&pool, book_id).await.unwrap();
    assert_eq!(
        trusted_book.resolution_outcome,
        Some(BookResolutionOutcome::Confirmed)
    );

    // Untrust — re-evaluate semantics: outcome cleared, status recomputed
    let untrusted_book = service.untrust_metadata(book_id).await.unwrap().unwrap();
    assert_eq!(
        untrusted_book.resolution_outcome, None,
        "untrust should clear outcome to None; got {:?}",
        untrusted_book.resolution_outcome
    );
    assert_eq!(
        untrusted_book.metadata_status,
        MetadataStatus::Identified,
        "book with applied candidate should recompute to Identified"
    );
}

/// Untrust re-evaluates: author-only book (no identifiers) recomputes to
/// `NeedsReview` with `resolution_outcome = None`.
#[tokio::test]
async fn trust_untrust_recomputes_author_only_to_needs_review() {
    let tmp = TempDir::new().unwrap();
    let (service, pool) = setup_no_provider(&tmp).await;

    let book = Book::new("Confirmed Book");
    BookRepository::create(&pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("Author B");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    // Manually set outcome to Confirmed (simulating a prior trust or apply)
    let mut confirmed_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    confirmed_book.resolution_outcome = Some(BookResolutionOutcome::Confirmed);
    confirmed_book.metadata_status = MetadataStatus::Identified;
    confirmed_book.resolution_state = archivis_core::models::ResolutionState::Done;
    BookRepository::update(&pool, &confirmed_book)
        .await
        .unwrap();

    // Trust
    let trusted = service.trust_metadata(book.id).await.unwrap();
    assert!(trusted, "trust should succeed");

    // Untrust — re-evaluate: outcome cleared, status recomputed from data
    let untrusted_book = service.untrust_metadata(book.id).await.unwrap().unwrap();
    assert_eq!(
        untrusted_book.resolution_outcome, None,
        "untrust should clear outcome to None; got {:?}",
        untrusted_book.resolution_outcome
    );
    assert_eq!(
        untrusted_book.metadata_status,
        MetadataStatus::NeedsReview,
        "author-only book (no identifiers) should recompute to NeedsReview"
    );
}

/// Untrust re-evaluates: empty book (no authors, no identifiers) recomputes
/// to `Unidentified` with `resolution_outcome = None`.
#[tokio::test]
async fn trust_untrust_recomputes_empty_to_unidentified() {
    let tmp = TempDir::new().unwrap();
    let (service, pool) = setup_no_provider(&tmp).await;

    let book = Book::new("Unmatched Book");
    BookRepository::create(&pool, &book).await.unwrap();

    // Set outcome to Unmatched (simulating a failed resolution)
    let mut unmatched_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    unmatched_book.resolution_outcome = Some(BookResolutionOutcome::Unmatched);
    unmatched_book.metadata_status = MetadataStatus::Unidentified;
    unmatched_book.resolution_state = archivis_core::models::ResolutionState::Done;
    BookRepository::update(&pool, &unmatched_book)
        .await
        .unwrap();

    // Trust
    let trusted = service.trust_metadata(book.id).await.unwrap();
    assert!(trusted, "trust should succeed");
    let trusted_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(
        trusted_book.resolution_outcome,
        Some(BookResolutionOutcome::Confirmed),
        "trust sets outcome to Confirmed"
    );

    // Untrust — re-evaluate: outcome cleared, status recomputed
    let untrusted_book = service.untrust_metadata(book.id).await.unwrap().unwrap();
    assert_eq!(
        untrusted_book.resolution_outcome, None,
        "untrust should clear outcome to None; got {:?}",
        untrusted_book.resolution_outcome
    );
    assert_eq!(
        untrusted_book.metadata_status,
        MetadataStatus::Unidentified,
        "empty book (no authors, no identifiers) should recompute to Unidentified"
    );
}

/// Untrust re-evaluates after trust during active review: book with applied
/// candidate recomputes to `Identified` with `resolution_outcome = None`.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn trust_during_review_untrust_recomputes_applied_candidate() {
    let tmp = TempDir::new().unwrap();

    // First resolve will auto-apply (strong ISBN), producing Enriched.
    // Second resolve (manual refresh) enters review with baseline=Enriched.
    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Review Book".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Author C".into(),
                role: Some("author".into()),
            }],
            description: Some("Description.".into()),
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780000000004".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.95,
        }],
        search_results: vec![],
    });

    let (service, pool) = setup_with_provider(&tmp, provider).await;

    // Initial resolve → auto-apply → Enriched
    let book = Book::new("Review Book");
    BookRepository::create(&pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("Author C");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let isbn = Identifier::new(
        book.id,
        IdentifierType::Isbn13,
        "9780000000004",
        MetadataSource::Embedded,
        0.9,
    );
    IdentifierRepository::create(&pool, &isbn).await.unwrap();

    let outcome1 = service.resolve_book(book.id, false).await.unwrap();
    assert!(outcome1.auto_applied, "precondition: should auto-apply");

    let post_resolve = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(
        post_resolve.resolution_outcome,
        Some(BookResolutionOutcome::Enriched),
        "precondition: should be Enriched after auto-apply with new fields"
    );

    // Manual refresh → enters review (baseline = Enriched, current = Ambiguous/Disputed)
    let _outcome2 = service.resolve_book(book.id, true).await.unwrap();
    let review_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert!(
        review_book.review_baseline_resolution_outcome.is_some(),
        "precondition: should have baseline outcome during review"
    );
    assert_eq!(
        review_book.review_baseline_resolution_outcome,
        Some(BookResolutionOutcome::Enriched),
        "precondition: baseline should be Enriched"
    );

    // Trust during review
    let trusted = service.trust_metadata(book.id).await.unwrap();
    assert!(trusted, "trust should succeed during review");

    // Untrust — re-evaluate: outcome cleared, status recomputed from data
    let untrusted_book = service.untrust_metadata(book.id).await.unwrap().unwrap();
    assert_eq!(
        untrusted_book.resolution_outcome, None,
        "untrust should clear outcome to None; got {:?}",
        untrusted_book.resolution_outcome
    );
    assert_eq!(
        untrusted_book.metadata_status,
        MetadataStatus::Identified,
        "book with applied candidate should recompute to Identified"
    );
}

/// Untrust re-evaluates after trust during active review: book with authors
/// and identifiers (no applied candidate) recomputes to `Identified` with
/// `resolution_outcome = None`.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn trust_during_review_untrust_recomputes_with_identifiers() {
    let tmp = TempDir::new().unwrap();

    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Different Title".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Author D".into(),
                role: Some("author".into()),
            }],
            description: None,
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780000000005".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.95,
        }],
        search_results: vec![],
    });

    let (service, pool) = setup_with_provider(&tmp, provider).await;

    // Create book with protected title. Set outcome to Confirmed directly
    // (simulating a prior user confirmation), NOT via trust_metadata, so
    // `metadata_user_trusted` stays false.
    let mut book = Book::new("My Title");
    book.metadata_provenance.title = Some(FieldProvenance {
        origin: MetadataSource::User,
        protected: true,
    });
    book.resolution_outcome = Some(BookResolutionOutcome::Confirmed);
    book.metadata_status = MetadataStatus::Identified;
    book.resolution_state = archivis_core::models::ResolutionState::Done;
    BookRepository::create(&pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("Author D");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let isbn = Identifier::new(
        book.id,
        IdentifierType::Isbn13,
        "9780000000005",
        MetadataSource::Embedded,
        0.9,
    );
    IdentifierRepository::create(&pool, &isbn).await.unwrap();

    // Re-resolve (manual refresh) → enters review with baseline = Confirmed
    let outcome = service.resolve_book(book.id, true).await.unwrap();
    assert!(
        !outcome.auto_applied,
        "protected title conflict → no auto-apply"
    );

    let review_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(
        review_book.review_baseline_resolution_outcome,
        Some(BookResolutionOutcome::Confirmed),
        "baseline should capture the pre-review Confirmed outcome"
    );
    // Current outcome is now the transient review value (Disputed or Ambiguous)
    assert!(
        matches!(
            review_book.resolution_outcome,
            Some(BookResolutionOutcome::Disputed | BookResolutionOutcome::Ambiguous)
        ),
        "precondition: review sets transient review outcome; got {:?}",
        review_book.resolution_outcome
    );

    // Trust during review (metadata_user_trusted = false → guard fires)
    let trusted = service.trust_metadata(book.id).await.unwrap();
    assert!(trusted);

    // Untrust — re-evaluate: outcome cleared, status recomputed from data
    let final_book = service.untrust_metadata(book.id).await.unwrap().unwrap();
    assert_eq!(
        final_book.resolution_outcome, None,
        "untrust should clear outcome to None; got {:?}",
        final_book.resolution_outcome
    );
    assert_eq!(
        final_book.metadata_status,
        MetadataStatus::Identified,
        "book with authors + identifiers should recompute to Identified"
    );
}

/// Regression: trust must supersede the active review run so
/// `latest_reviewable_run_id` no longer returns stale state.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn trust_supersedes_active_review_run() {
    let tmp = TempDir::new().unwrap();

    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Supersede Book".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Author E".into(),
                role: Some("author".into()),
            }],
            description: Some("Desc.".into()),
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![ProviderIdentifier {
                identifier_type: IdentifierType::Isbn13,
                value: "9780000000006".into(),
            }],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.95,
        }],
        search_results: vec![],
    });

    let (service, pool) = setup_with_provider(&tmp, provider).await;

    let book = Book::new("Supersede Book");
    BookRepository::create(&pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("Author E");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let isbn = Identifier::new(
        book.id,
        IdentifierType::Isbn13,
        "9780000000006",
        MetadataSource::Embedded,
        0.9,
    );
    IdentifierRepository::create(&pool, &isbn).await.unwrap();

    // Initial resolve → auto-apply
    let outcome1 = service.resolve_book(book.id, false).await.unwrap();
    assert!(outcome1.auto_applied);

    // Manual refresh → enters review, creates a new run
    let _outcome2 = service.resolve_book(book.id, true).await.unwrap();

    // Verify there's an active review run
    let mut conn = pool.acquire().await.unwrap();
    let review_run_id = CandidateRepository::latest_reviewable_run_id_conn(conn.as_mut(), book.id)
        .await
        .unwrap();
    assert!(
        review_run_id.is_some(),
        "precondition: should have active review run before trust"
    );
    let run_id = review_run_id.unwrap();
    drop(conn);

    // Trust → should supersede the review run
    let trusted = service.trust_metadata(book.id).await.unwrap();
    assert!(trusted);

    // Verify: review run is superseded
    let run_after = ResolutionRunRepository::get_by_id(&pool, run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        run_after.state,
        archivis_core::models::ResolutionRunState::Superseded,
        "review run should be superseded after trust; got {:?}",
        run_after.state
    );

    // Verify: `latest_reviewable_run_id` no longer returns the run
    let mut conn2 = pool.acquire().await.unwrap();
    let stale_run = CandidateRepository::latest_reviewable_run_id_conn(conn2.as_mut(), book.id)
        .await
        .unwrap();
    assert!(
        stale_run.is_none(),
        "latest_reviewable_run_id should return None after trust superseded the run"
    );
}

/// Untrust after author removal while trusted reflects current data.
///
/// Book starts with author + identifier → Identified. Trust. Remove the
/// author while trusted. Untrust. Verify outcome=None, status=NeedsReview
/// (has identifier but no author).
#[tokio::test]
async fn untrust_after_author_removal_reflects_changed_snapshot() {
    let tmp = TempDir::new().unwrap();
    let (service, pool) = setup_no_provider(&tmp).await;

    let book = Book::new("Author Removal Book");
    BookRepository::create(&pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("Removable Author");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let isbn = Identifier::new(
        book.id,
        IdentifierType::Isbn13,
        "9780000000010",
        MetadataSource::Embedded,
        0.9,
    );
    IdentifierRepository::create(&pool, &isbn).await.unwrap();

    // Set to Identified/Done (simulating prior resolution)
    let mut setup_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    setup_book.metadata_status = MetadataStatus::Identified;
    setup_book.resolution_outcome = Some(BookResolutionOutcome::Confirmed);
    setup_book.resolution_state = archivis_core::models::ResolutionState::Done;
    BookRepository::update(&pool, &setup_book).await.unwrap();

    // Trust
    let trusted = service.trust_metadata(book.id).await.unwrap();
    assert!(trusted, "trust should succeed");

    // Remove author while trusted
    BookRepository::clear_authors(&pool, book.id).await.unwrap();

    // Untrust — should reflect post-edit data (identifier only, no author)
    let untrusted_book = service.untrust_metadata(book.id).await.unwrap().unwrap();
    assert_eq!(
        untrusted_book.resolution_outcome, None,
        "untrust should clear outcome to None; got {:?}",
        untrusted_book.resolution_outcome
    );
    assert_eq!(
        untrusted_book.metadata_status,
        MetadataStatus::NeedsReview,
        "identifier-only book (author removed) should recompute to NeedsReview"
    );
}

/// Regression test for the original bug path:
/// `NeedsReview → reject all → trust → untrust` must not restore a stale
/// outcome. Outcome is cleared to None, status recomputed from data.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn untrust_original_bug_path_needs_review_with_identifiers() {
    let tmp = TempDir::new().unwrap();

    // Provider returns a candidate that will NOT auto-apply (no ISBN in result,
    // only title match — below threshold for auto-apply without hard ID proof).
    let provider = Arc::new(FlexibleStubProvider {
        isbn_results: vec![],
        search_results: vec![ProviderMetadata {
            provider_name: "test_provider".into(),
            title: Some("Bug Path Book".into()),
            subtitle: None,
            authors: vec![ProviderAuthor {
                name: "Author F".into(),
                role: Some("author".into()),
            }],
            description: None,
            language: None,
            publisher: None,
            publication_year: None,
            identifiers: vec![],
            subjects: vec![],
            series: None,
            page_count: None,
            cover_url: None,
            rating: None,
            physical_format: None,
            confidence: 0.7,
        }],
    });

    let (service, pool) = setup_with_provider(&tmp, provider).await;

    let book = Book::new("Bug Path Book");
    BookRepository::create(&pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("Author F");
    AuthorRepository::create(&pool, &author).await.unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    // Add a non-ISBN identifier (e.g., ASIN) — won't trigger ISBN lookup
    let asin = Identifier::new(
        book.id,
        IdentifierType::Asin,
        "B00TEST1234",
        MetadataSource::Embedded,
        0.9,
    );
    IdentifierRepository::create(&pool, &asin).await.unwrap();

    // Resolve → enters review (candidate not auto-applied)
    let outcome = service.resolve_book(book.id, false).await.unwrap();
    assert!(
        !outcome.auto_applied,
        "precondition: should NOT auto-apply without strong ID proof"
    );

    let review_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(
        review_book.metadata_status,
        MetadataStatus::NeedsReview,
        "precondition: should be NeedsReview with pending candidates"
    );

    // Reject all candidates
    let candidates = CandidateRepository::list_by_book(&pool, book.id)
        .await
        .unwrap();
    for c in &candidates {
        if c.status == CandidateStatus::Pending {
            CandidateRepository::update_status(&pool, c.id, CandidateStatus::Rejected)
                .await
                .unwrap();
        }
    }

    // Trust — sets Identified/Confirmed
    let trusted = service.trust_metadata(book.id).await.unwrap();
    assert!(trusted, "trust should succeed");

    let trusted_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(
        trusted_book.resolution_outcome,
        Some(BookResolutionOutcome::Confirmed)
    );

    // Untrust — the fix: outcome is cleared, status recomputed from data
    let untrusted_book = service.untrust_metadata(book.id).await.unwrap().unwrap();
    assert_eq!(
        untrusted_book.resolution_outcome, None,
        "untrust should clear outcome to None (not restore stale snapshot); got {:?}",
        untrusted_book.resolution_outcome
    );
    // Book has authors + identifiers (ASIN) → `recompute_status` → Identified
    assert_eq!(
        untrusted_book.metadata_status,
        MetadataStatus::Identified,
        "book with authors + identifiers should recompute to Identified"
    );
}

/// Locked trusted book: untrust keeps `resolution_state = Done`, book
/// doesn't enter auto-resolution queue.
#[tokio::test]
async fn locked_trusted_book_untrust_stays_done() {
    let tmp = TempDir::new().unwrap();
    let (service, pool) = setup_no_provider(&tmp).await;

    let book = Book::new("Locked Book");
    BookRepository::create(&pool, &book).await.unwrap();

    // Trust first
    let trusted = service.trust_metadata(book.id).await.unwrap();
    assert!(trusted);

    // Set `metadata_locked = true`
    let mut locked_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    locked_book.metadata_locked = true;
    BookRepository::update(&pool, &locked_book).await.unwrap();

    // Untrust
    let untrusted_book = service.untrust_metadata(book.id).await.unwrap().unwrap();
    assert_eq!(
        untrusted_book.resolution_outcome, None,
        "untrust should clear outcome to None"
    );
    assert_eq!(
        untrusted_book.resolution_state,
        archivis_core::models::ResolutionState::Done,
        "locked book should stay Done (Pending→Done conversion), not enter resolution queue"
    );
    assert!(
        untrusted_book.metadata_locked,
        "metadata_locked should be preserved"
    );
}
