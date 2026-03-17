//! Integration tests for cover handling during resolution candidate application.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use archivis_core::models::{Book, BookFile, BookFormat, IdentificationCandidate};
use archivis_core::settings::SettingsReader;
use archivis_db::{BookFileRepository, BookRepository, CandidateRepository};
use archivis_metadata::{
    MetadataProvider, MetadataQuery, MetadataResolver, ProviderAuthor, ProviderMetadata,
    ProviderRegistry,
};
use archivis_storage::local::LocalStorage;
use archivis_storage::StorageBackend;
use archivis_tasks::resolve::ResolutionService;
use tempfile::TempDir;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Minimal JPEG bytes (a valid 1x1 red JPEG).
fn tiny_jpeg() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0]));
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
    buf.into_inner()
}

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

/// Build a `ProviderMetadata` with a cover URL.
fn provider_meta_with_cover(cover_url: &str) -> ProviderMetadata {
    ProviderMetadata {
        provider_name: "test_provider".into(),
        title: Some("Ready for Anything".into()),
        subtitle: None,
        authors: vec![ProviderAuthor {
            name: "David Allen".into(),
            role: None,
        }],
        description: Some("A test book".into()),
        language: None,
        publisher: None,
        publication_year: None,
        identifiers: vec![],
        subjects: vec![],
        series: None,
        page_count: None,
        cover_url: Some(cover_url.into()),
        rating: None,
        physical_format: None,
        confidence: 0.95,
        merged_from: Vec::new(),
        field_sources: BTreeMap::new(),
    }
}

/// Set up a test environment: DB, storage, and identification service.
async fn setup(
    tmp: &TempDir,
    mock_server: Option<&MockServer>,
) -> (
    ResolutionService<LocalStorage>,
    archivis_db::DbPool,
    LocalStorage,
) {
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
    if let Some(server) = mock_server {
        registry.register(Arc::new(StubProvider::new(server.uri())));
    }

    let resolver = Arc::new(MetadataResolver::new(Arc::new(registry), settings));
    let service = ResolutionService::new(pool.clone(), resolver, storage.clone(), data_dir);

    (service, pool, storage)
}

/// Create a book with a candidate that has a cover URL, and return both IDs.
async fn create_book_with_candidate(
    pool: &archivis_db::DbPool,
    cover_url: &str,
    existing_cover: Option<&str>,
) -> (uuid::Uuid, uuid::Uuid) {
    let mut book = Book::new("Ready for Anything");
    book.cover_path = existing_cover.map(String::from);
    BookRepository::create(pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("Unknown Author");
    archivis_db::AuthorRepository::create(pool, &author)
        .await
        .unwrap();
    BookRepository::add_author(pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let meta = provider_meta_with_cover(cover_url);
    let meta_json = serde_json::to_value(&meta).unwrap();
    let candidate = IdentificationCandidate::new(book.id, "test_provider", 0.95, meta_json, vec![]);
    CandidateRepository::create(pool, &candidate).await.unwrap();

    (book.id, candidate.id)
}

/// Store a fake cover file in storage and return its path.
async fn store_fake_cover(storage: &LocalStorage, book_dir: &str) -> String {
    let path = format!("{book_dir}/cover.jpg");
    storage.store(&path, &tiny_jpeg()).await.unwrap();
    path
}

// ── Stub MetadataProvider for auto-apply test ───────────────────────

struct StubProvider {
    base_url: String,
}

impl StubProvider {
    fn new(base_url: String) -> Self {
        Self { base_url }
    }
}

static COVER_STUB_CAPS: archivis_metadata::ProviderCapabilities =
    archivis_metadata::ProviderCapabilities {
        quality: archivis_metadata::ProviderQuality::Community,
        default_rate_limit_rpm: 100,
        supported_id_lookups: &[
            archivis_core::models::IdentifierType::Isbn13,
            archivis_core::models::IdentifierType::Isbn10,
            archivis_core::models::IdentifierType::Asin,
        ],
        features: &[
            archivis_metadata::ProviderFeature::Search,
            archivis_metadata::ProviderFeature::Covers,
        ],
    };

#[async_trait::async_trait]
impl MetadataProvider for StubProvider {
    fn name(&self) -> &'static str {
        "test_provider"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn capabilities(&self) -> &'static archivis_metadata::ProviderCapabilities {
        &COVER_STUB_CAPS
    }

    async fn lookup_isbn(
        &self,
        _isbn: &str,
    ) -> Result<Vec<ProviderMetadata>, archivis_metadata::ProviderError> {
        Ok(vec![provider_meta_with_cover(&format!(
            "{}/cover.jpg",
            self.base_url
        ))])
    }

    async fn search(
        &self,
        _query: &MetadataQuery,
    ) -> Result<Vec<ProviderMetadata>, archivis_metadata::ProviderError> {
        Ok(vec![provider_meta_with_cover(&format!(
            "{}/cover.jpg",
            self.base_url
        ))])
    }

    async fn fetch_cover(
        &self,
        cover_url: &str,
    ) -> Result<Vec<u8>, archivis_metadata::ProviderError> {
        let bytes = reqwest::get(cover_url)
            .await
            .map_err(archivis_metadata::ProviderError::HttpError)?
            .bytes()
            .await
            .map_err(archivis_metadata::ProviderError::HttpError)?;
        Ok(bytes.to_vec())
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[tokio::test]
async fn cover_applied_when_book_has_no_cover() {
    let tmp = TempDir::new().unwrap();
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(tiny_jpeg())
                .insert_header("content-type", "image/jpeg"),
        )
        .mount(&mock_server)
        .await;

    let (service, pool, _storage) = setup(&tmp, None).await;
    let cover_url = format!("{}/cover.jpg", mock_server.uri());
    let (book_id, candidate_id) = create_book_with_candidate(&pool, &cover_url, None).await;

    let updated = service
        .apply_candidate(book_id, candidate_id, &HashSet::new())
        .await
        .unwrap();

    assert!(
        updated.cover_path.is_some(),
        "cover_path should be set when book had no cover"
    );
}

#[tokio::test]
async fn cover_replaced_when_user_enables_toggle() {
    let tmp = TempDir::new().unwrap();
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(tiny_jpeg())
                .insert_header("content-type", "image/jpeg"),
        )
        .mount(&mock_server)
        .await;

    let (service, pool, storage) = setup(&tmp, None).await;

    // Store an old cover
    let old_cover = store_fake_cover(&storage, "D/David Allen/Old Book").await;

    let cover_url = format!("{}/cover.jpg", mock_server.uri());
    let (book_id, candidate_id) =
        create_book_with_candidate(&pool, &cover_url, Some(&old_cover)).await;

    // Add a book file in a *different* directory so the new cover
    // goes somewhere else, proving replacement actually happened.
    let book_file = BookFile::new(
        book_id,
        BookFormat::Epub,
        "D/David Allen/Ready for Anything/book.epub",
        1000,
        "abcdef12".repeat(8),
        None,
    );
    BookFileRepository::create(&pool, &book_file).await.unwrap();

    // preserve_existing_cover = false → user wants to replace
    let updated = service
        .apply_candidate(book_id, candidate_id, &HashSet::new())
        .await
        .unwrap();

    assert!(
        updated.cover_path.is_some(),
        "cover_path should be set after replacement"
    );
    assert_ne!(
        updated.cover_path.as_deref(),
        Some(old_cover.as_str()),
        "cover_path should point to new location (book file dir)"
    );

    // Old cover should be deleted from storage
    assert!(
        !storage.exists(&old_cover).await.unwrap_or(true),
        "old cover file should be deleted from storage"
    );
}

#[tokio::test]
async fn cover_replaced_in_same_directory() {
    let tmp = TempDir::new().unwrap();
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(tiny_jpeg())
                .insert_header("content-type", "image/jpeg"),
        )
        .mount(&mock_server)
        .await;

    let (service, pool, storage) = setup(&tmp, None).await;

    // Book file and old cover live in the SAME directory — the most common
    // case.  The new cover overwrites in-place; the cleanup must NOT delete
    // the file we just stored.
    let book_dir = "D/David Allen/Ready for Anything";
    let old_cover = store_fake_cover(&storage, book_dir).await;

    let cover_url = format!("{}/cover.jpg", mock_server.uri());
    let (book_id, candidate_id) =
        create_book_with_candidate(&pool, &cover_url, Some(&old_cover)).await;

    let book_file = BookFile::new(
        book_id,
        BookFormat::Epub,
        format!("{book_dir}/book.epub"),
        1000,
        "aabbccdd".repeat(8),
        None,
    );
    BookFileRepository::create(&pool, &book_file).await.unwrap();

    let updated = service
        .apply_candidate(book_id, candidate_id, &HashSet::new())
        .await
        .unwrap();

    // Path stays the same (same directory, same filename)
    assert_eq!(
        updated.cover_path.as_deref(),
        Some(old_cover.as_str()),
        "cover_path should remain at the same location"
    );

    // The file must still exist — it was overwritten in place, NOT deleted
    assert!(
        storage.exists(&old_cover).await.unwrap(),
        "cover file must exist after in-place replacement"
    );
}

#[tokio::test]
async fn cover_preserved_when_user_disables_toggle() {
    let tmp = TempDir::new().unwrap();
    let (service, pool, storage) = setup(&tmp, None).await;

    // Store an existing cover
    let old_cover = store_fake_cover(&storage, "D/David Allen/Ready").await;

    // The cover URL doesn't matter — it won't be fetched
    let (book_id, candidate_id) =
        create_book_with_candidate(&pool, "http://unused/cover.jpg", Some(&old_cover)).await;

    // "cover" in exclude_fields → user unchecked the cover toggle
    let mut exclude = HashSet::new();
    exclude.insert("cover".to_string());

    let updated = service
        .apply_candidate(book_id, candidate_id, &exclude)
        .await
        .unwrap();

    assert_eq!(
        updated.cover_path.as_deref(),
        Some(old_cover.as_str()),
        "cover_path should be unchanged when cover is excluded"
    );

    // Old cover file must still exist
    assert!(
        storage.exists(&old_cover).await.unwrap(),
        "old cover file should still exist"
    );
}

#[tokio::test]
async fn auto_apply_preserves_existing_cover() {
    let tmp = TempDir::new().unwrap();
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(tiny_jpeg())
                .insert_header("content-type", "image/jpeg"),
        )
        .mount(&mock_server)
        .await;

    let (service, pool, storage) = setup(&tmp, Some(&mock_server)).await;

    // Create book with an existing cover and an ISBN for lookup
    let old_cover = store_fake_cover(&storage, "D/David Allen/Ready for Anything").await;
    let mut book = Book::new("Ready for Anything");
    book.cover_path = Some(old_cover.clone());
    BookRepository::create(&pool, &book).await.unwrap();

    // Add an author so identification works
    let author = archivis_core::models::Author::new("David Allen");
    archivis_db::AuthorRepository::create(&pool, &author)
        .await
        .unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    // Add an ISBN identifier so the provider lookup triggers
    let isbn = archivis_core::models::Identifier::new(
        book.id,
        archivis_core::models::IdentifierType::Isbn13,
        "9780143126683",
        archivis_core::models::MetadataSource::Embedded,
        0.9,
    );
    archivis_db::IdentifierRepository::create(&pool, &isbn)
        .await
        .unwrap();

    // Run full identification (which auto-applies if dominant)
    let outcome = service.resolve_book(book.id, false).await.unwrap();
    assert!(
        !outcome.resolver_result.candidates.is_empty(),
        "should have at least one candidate"
    );

    // Reload book from DB
    let updated = BookRepository::get_by_id(&pool, book.id).await.unwrap();

    assert_eq!(
        updated.cover_path.as_deref(),
        Some(old_cover.as_str()),
        "auto-apply should preserve existing cover"
    );

    // Old cover file must still exist
    assert!(
        storage.exists(&old_cover).await.unwrap(),
        "old cover file should still exist after auto-apply"
    );
}

#[tokio::test]
async fn cover_fetch_failure_preserves_old_cover() {
    let tmp = TempDir::new().unwrap();
    let mock_server = MockServer::start().await;

    // Return 500 for cover fetch
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let (service, pool, storage) = setup(&tmp, None).await;

    // Store an existing cover
    let old_cover = store_fake_cover(&storage, "D/David Allen/Ready").await;

    let cover_url = format!("{}/cover.jpg", mock_server.uri());
    let (book_id, candidate_id) =
        create_book_with_candidate(&pool, &cover_url, Some(&old_cover)).await;

    // Try to apply — cover fetch will fail with 500
    let updated = service
        .apply_candidate(book_id, candidate_id, &HashSet::new())
        .await
        .unwrap();

    // Candidate should still be applied (metadata merged)
    assert_eq!(
        updated.cover_path.as_deref(),
        Some(old_cover.as_str()),
        "old cover should be preserved when fetch fails"
    );

    // Old cover file must still exist
    assert!(
        storage.exists(&old_cover).await.unwrap(),
        "old cover file should still exist after failed fetch"
    );
}

#[tokio::test]
async fn cover_stored_in_book_file_directory() {
    let tmp = TempDir::new().unwrap();
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(tiny_jpeg())
                .insert_header("content-type", "image/jpeg"),
        )
        .mount(&mock_server)
        .await;

    let (service, pool, _storage) = setup(&tmp, None).await;

    let cover_url = format!("{}/cover.jpg", mock_server.uri());
    let (book_id, candidate_id) = create_book_with_candidate(&pool, &cover_url, None).await;

    // Add a book file so the cover path uses its directory
    let book_file = BookFile::new(
        book_id,
        BookFormat::Epub,
        "D/David Allen/Ready for Anything/book.epub",
        1000,
        "abcdef12".repeat(8),
        None,
    );
    BookFileRepository::create(&pool, &book_file).await.unwrap();

    let updated = service
        .apply_candidate(book_id, candidate_id, &HashSet::new())
        .await
        .unwrap();

    let cover_path = updated.cover_path.unwrap();
    assert!(
        cover_path.starts_with("D/David Allen/Ready for Anything/"),
        "cover should be stored in the book file's directory, got: {cover_path}"
    );
    assert!(
        !cover_path.contains("Unknown Author"),
        "cover path should not contain 'Unknown Author', got: {cover_path}"
    );
}

#[tokio::test]
async fn cover_uses_existing_cover_directory_when_no_book_files() {
    let tmp = TempDir::new().unwrap();
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(tiny_jpeg())
                .insert_header("content-type", "image/jpeg"),
        )
        .mount(&mock_server)
        .await;

    let (service, pool, storage) = setup(&tmp, None).await;
    let existing_cover = store_fake_cover(&storage, "D/David Allen/Existing Dir").await;

    let cover_url = format!("{}/cover.jpg", mock_server.uri());
    let (book_id, candidate_id) =
        create_book_with_candidate(&pool, &cover_url, Some(&existing_cover)).await;

    let updated = service
        .apply_candidate(book_id, candidate_id, &HashSet::new())
        .await
        .unwrap();

    let cover_path = updated.cover_path.unwrap();
    assert!(
        cover_path.starts_with("D/David Allen/Existing Dir/"),
        "cover should reuse existing cover directory, got: {cover_path}"
    );
}

#[tokio::test]
async fn cover_fallback_uses_real_author_when_no_files_or_existing_cover() {
    let tmp = TempDir::new().unwrap();
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(tiny_jpeg())
                .insert_header("content-type", "image/jpeg"),
        )
        .mount(&mock_server)
        .await;

    let (service, pool, _storage) = setup(&tmp, None).await;

    let book = Book::new("Ready for Anything");
    BookRepository::create(&pool, &book).await.unwrap();

    let author = archivis_core::models::Author::new("David Allen");
    archivis_db::AuthorRepository::create(&pool, &author)
        .await
        .unwrap();
    BookRepository::add_author(&pool, book.id, author.id, "author", 0)
        .await
        .unwrap();

    let meta = provider_meta_with_cover(&format!("{}/cover.jpg", mock_server.uri()));
    let candidate = IdentificationCandidate::new(
        book.id,
        "test_provider",
        0.95,
        serde_json::to_value(meta).unwrap(),
        vec![],
    );
    CandidateRepository::create(&pool, &candidate)
        .await
        .unwrap();

    let updated = service
        .apply_candidate(book.id, candidate.id, &HashSet::new())
        .await
        .unwrap();

    let cover_path = updated.cover_path.unwrap();
    assert!(
        cover_path.starts_with("D/David Allen/Ready for Anything/"),
        "fallback path should use real author name, got: {cover_path}"
    );
    assert!(
        !cover_path.contains("Unknown Author"),
        "fallback path should not contain 'Unknown Author', got: {cover_path}"
    );
}

#[tokio::test]
async fn cover_replace_keeps_fresh_thumbnails() {
    let tmp = TempDir::new().unwrap();
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(tiny_jpeg())
                .insert_header("content-type", "image/jpeg"),
        )
        .mount(&mock_server)
        .await;

    let (service, pool, storage) = setup(&tmp, None).await;
    let old_cover = store_fake_cover(&storage, "D/David Allen/Ready for Anything").await;
    let cover_url = format!("{}/cover.jpg", mock_server.uri());
    let (book_id, candidate_id) =
        create_book_with_candidate(&pool, &cover_url, Some(&old_cover)).await;

    let book_file = BookFile::new(
        book_id,
        BookFormat::Epub,
        "D/David Allen/Ready for Anything/book.epub",
        1000,
        "ff00aa11".repeat(8),
        None,
    );
    BookFileRepository::create(&pool, &book_file).await.unwrap();

    // Seed a stale cache file to ensure the directory is invalidated.
    let cache_dir = tmp
        .path()
        .join("data")
        .join("covers")
        .join(book_id.to_string());
    tokio::fs::create_dir_all(&cache_dir).await.unwrap();
    tokio::fs::write(cache_dir.join("lg.webp"), b"stale")
        .await
        .unwrap();

    service
        .apply_candidate(book_id, candidate_id, &HashSet::new())
        .await
        .unwrap();

    assert!(
        cache_dir.join("sm.webp").exists(),
        "sm thumbnail should exist after cover replacement"
    );
    assert!(
        cache_dir.join("md.webp").exists(),
        "md thumbnail should exist after cover replacement"
    );
    assert!(
        !cache_dir.join("lg.webp").exists(),
        "stale lg thumbnail should be removed when cache is invalidated"
    );
}
