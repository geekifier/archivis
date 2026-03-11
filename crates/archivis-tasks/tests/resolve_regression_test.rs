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
use archivis_db::{AuthorRepository, BookRepository, CandidateRepository, IdentifierRepository};
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

    // 2. Keep current metadata to set Confirmed outcome
    service.keep_current_metadata(book.id).await.unwrap();
    let confirmed_book = BookRepository::get_by_id(&pool, book.id).await.unwrap();
    assert_eq!(
        confirmed_book.resolution_outcome,
        Some(BookResolutionOutcome::Confirmed),
        "precondition: should be Confirmed after keep_current_metadata"
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
