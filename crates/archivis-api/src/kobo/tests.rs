//! End-to-end tests for the Kobo Sync API and protocol routes.

#![cfg(test)]

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Write};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http::Method;
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

use archivis_auth::{AuthService, LocalAuthAdapter};
use archivis_core::models::{Book, BookFile, BookFormat};
use archivis_core::public_url::PublicBaseUrl;
use archivis_db::{
    create_pool, run_migrations, BookFileRepository, BookRepository, KoboSyncSelectionRepository,
};
use archivis_formats::transform::TransformerRegistry;
use archivis_metadata::{MetadataResolver, ProviderRegistry};
use archivis_storage::local::LocalStorage;
use archivis_storage::StorageBackend;
use archivis_tasks::{merge::MergeService, queue::TaskQueue, resolve::ResolutionService};

use crate::kobo::types::{
    DeviceResponse, KoboStatusResponse, KoboSyncStateResponse, PairDeviceResponse,
};
use crate::settings::service::ConfigService;
use crate::state::{ApiConfig, AppState};

struct TestSettings;
impl archivis_core::settings::SettingsReader for TestSettings {
    fn get_setting(&self, _key: &str) -> Option<serde_json::Value> {
        None
    }
}

fn build_test_epub() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut out));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/epub+zip").unwrap();

        zip.start_file("META-INF/container.xml", deflated).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#,
        )
        .unwrap();

        zip.start_file("OEBPS/content.opf", deflated).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Fixture</dc:title>
    <dc:identifier id="uid">urn:uuid:fixture</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#,
        )
        .unwrap();

        zip.start_file("OEBPS/ch1.xhtml", deflated).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head><body><p>Hello.</p></body></html>"#,
        )
        .unwrap();
        zip.finish().unwrap();
    }
    out
}

async fn test_state(tmp: &TempDir, public_base_url: Option<&str>) -> AppState {
    let db_path = tmp.path().join("kobo.db");
    let storage_dir = tmp.path().join("books");

    let db_pool = create_pool(&db_path).await.unwrap();
    run_migrations(&db_pool).await.unwrap();

    let storage = LocalStorage::new(&storage_dir).await.unwrap();
    let auth_adapter = LocalAuthAdapter::new(db_pool.clone());
    let auth_service = AuthService::new(db_pool.clone(), auth_adapter);
    let (task_queue, mut rx) = TaskQueue::new(db_pool.clone());
    tokio::spawn(async move { while rx.recv().await.is_some() {} });

    let provider_registry = Arc::new(ProviderRegistry::new());
    let resolver = Arc::new(MetadataResolver::new(
        Arc::clone(&provider_registry),
        Arc::new(TestSettings),
    ));
    let resolve_service = Arc::new(ResolutionService::new(
        db_pool.clone(),
        resolver,
        storage.clone(),
        tmp.path().to_path_buf(),
    ));
    let merge_service = Arc::new(MergeService::new(
        db_pool.clone(),
        storage.clone(),
        tmp.path().to_path_buf(),
    ));
    let config_service = Arc::new(ConfigService::for_tests(db_pool.clone()));
    let transformers = TransformerRegistry::new(vec![Arc::new(archivis_kepub::KepubTransformer)
        as Arc<dyn archivis_formats::transform::FormatTransformer>]);

    AppState::new(
        db_pool,
        Arc::new(task_queue),
        auth_service,
        storage,
        provider_registry,
        resolve_service,
        merge_service,
        ApiConfig {
            data_dir: tmp.path().to_path_buf(),
            frontend_dir: None,
            public_base_url: public_base_url.map(|u| PublicBaseUrl::parse(u).unwrap()),
        },
        config_service,
        Arc::new(transformers),
        None,
        None,
        [0u8; 32],
    )
}

async fn register_and_login(state: &AppState, name: &str) -> String {
    state
        .auth_service()
        .register(name, "passw0rd-strong", None)
        .await
        .unwrap();
    let (token, _) = state
        .auth_service()
        .login(name, "passw0rd-strong")
        .await
        .unwrap();
    token
}

async fn seed_epub_book(state: &AppState) -> (Uuid, Uuid) {
    let pool = state.db_pool();
    let book = Book::new("Hello World");
    BookRepository::create(pool, &book).await.unwrap();
    let bytes = build_test_epub();
    let stored = state
        .storage()
        .store(&format!("h/{}.epub", book.id), &bytes)
        .await
        .unwrap();
    let file = BookFile::new(
        book.id,
        BookFormat::Epub,
        &stored.path,
        #[allow(clippy::cast_possible_wrap)]
        (stored.size as i64),
        &stored.hash,
        None,
    );
    BookFileRepository::create(pool, &file).await.unwrap();
    (book.id, file.id)
}

async fn seed_unique_epub_book(state: &AppState, title: &str) -> (Uuid, Uuid) {
    let pool = state.db_pool();
    let book = Book::new(title);
    BookRepository::create(pool, &book).await.unwrap();
    let mut bytes = build_test_epub();
    bytes.extend_from_slice(title.as_bytes());
    let stored = state
        .storage()
        .store(&format!("h/{}.epub", book.id), &bytes)
        .await
        .unwrap();
    let file = BookFile::new(
        book.id,
        BookFormat::Epub,
        &stored.path,
        #[allow(clippy::cast_possible_wrap)]
        (stored.size as i64),
        &stored.hash,
        None,
    );
    BookFileRepository::create(pool, &file).await.unwrap();
    (book.id, file.id)
}

async fn seed_cover(state: &AppState, book_id: Uuid) {
    let cover = state
        .storage()
        .store(&format!("covers/{book_id}/cover.jpg"), b"fake-jpeg")
        .await
        .unwrap();
    let mut book = BookRepository::get_by_id(state.db_pool(), book_id)
        .await
        .unwrap();
    book.cover_path = Some(cover.path);
    BookRepository::update(state.db_pool(), &book)
        .await
        .unwrap();
}

async fn seed_pdf_only_book(state: &AppState) -> Uuid {
    let pool = state.db_pool();
    let book = Book::new("PDF only");
    BookRepository::create(pool, &book).await.unwrap();
    let file = BookFile::new(
        book.id,
        BookFormat::Pdf,
        format!("{}.pdf", book.id),
        100,
        format!("hash-pdf-{}", book.id),
        None,
    );
    BookFileRepository::create(pool, &file).await.unwrap();
    book.id
}

async fn json_body<T: serde::de::DeserializeOwned>(resp: axum::http::Response<Body>) -> T {
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap_or_else(|e| {
        panic!(
            "failed to deserialize body: {e}\nbody: {}",
            String::from_utf8_lossy(&body)
        )
    })
}

fn req(method: Method, uri: &str, token: Option<&str>, body: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    if let Some(b) = body {
        builder = builder.header("content-type", "application/json");
        builder.body(Body::from(b.to_string())).unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    }
}

// ── User-facing API tests ────────────────────────────────────────────

#[tokio::test]
async fn pairing_requires_public_base_url() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, None).await;
    let token = register_and_login(&state, "alice").await;
    let app = crate::build_router(state);

    let resp = app
        .oneshot(req(
            Method::POST,
            "/api/kobo/devices",
            Some(&token),
            Some("{}"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn pairing_returns_token_once_and_lists_without_token() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let token = register_and_login(&state, "alice").await;
    let app = crate::build_router(state);

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/kobo/devices",
            Some(&token),
            Some(r#"{"display_name":"Libra"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let pair: PairDeviceResponse = json_body(resp).await;
    assert_eq!(pair.display_name, "Libra");
    assert!(pair.api_endpoint.contains("/kobo/"));
    assert_eq!(pair.token.len(), 64);

    // Listing must never include the raw token.
    let list_resp = app
        .clone()
        .oneshot(req(Method::GET, "/api/kobo/devices", Some(&token), None))
        .await
        .unwrap();
    assert_eq!(list_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(list_resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let raw = String::from_utf8(body.to_vec()).unwrap();
    assert!(!raw.contains(&pair.token), "list response leaked the token");
    let devices: Vec<DeviceResponse> = serde_json::from_str(&raw).unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id, pair.id);
}

#[tokio::test]
async fn revocation_is_idempotent_from_users_pov() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let token = register_and_login(&state, "alice").await;
    let app = crate::build_router(state);

    let pair_resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/kobo/devices",
            Some(&token),
            Some("{}"),
        ))
        .await
        .unwrap();
    let pair: PairDeviceResponse = json_body(pair_resp).await;

    let url = format!("/api/kobo/devices/{}", pair.id);
    let r1 = app
        .clone()
        .oneshot(req(Method::DELETE, &url, Some(&token), None))
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::NO_CONTENT);
    let r2 = app
        .oneshot(req(Method::DELETE, &url, Some(&token), None))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn book_sync_toggle_rejects_no_epub_book() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let token = register_and_login(&state, "alice").await;
    let book_id = seed_pdf_only_book(&state).await;
    let app = crate::build_router(state);

    let resp = app
        .oneshot(req(
            Method::PUT,
            &format!("/api/books/{book_id}/kobo-sync"),
            Some(&token),
            Some(r#"{"enabled":true}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn book_detail_includes_kobo_sync_state() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let token = register_and_login(&state, "alice").await;
    let (book_id, file_id) = seed_epub_book(&state).await;
    let app = crate::build_router(state);

    // Initially: not enabled.
    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/api/books/{book_id}"),
            Some(&token),
            None,
        ))
        .await
        .unwrap();
    let detail: serde_json::Value = json_body(resp).await;
    let ks = &detail["kobo_sync"];
    assert_eq!(ks["enabled"], false);
    assert_eq!(
        ks["eligible_file_ids"].as_array().unwrap()[0],
        file_id.to_string()
    );

    // Enable it.
    let put = app
        .clone()
        .oneshot(req(
            Method::PUT,
            &format!("/api/books/{book_id}/kobo-sync"),
            Some(&token),
            Some(r#"{"enabled":true}"#),
        ))
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    let state_resp: KoboSyncStateResponse = json_body(put).await;
    assert!(state_resp.enabled);
    assert_eq!(state_resp.selected_book_file_id, Some(file_id));
}

// ── Protocol tests ────────────────────────────────────────────────────

async fn pair_and_select_epub(app: &axum::Router, auth_token: &str) -> (PairDeviceResponse, Uuid) {
    let pair_resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/kobo/devices",
            Some(auth_token),
            Some("{}"),
        ))
        .await
        .unwrap();
    assert_eq!(pair_resp.status(), StatusCode::CREATED);
    let pair: PairDeviceResponse = json_body(pair_resp).await;
    (pair, Uuid::nil())
}

async fn update_runtime_setting(state: &AppState, key: &str, value: serde_json::Value) {
    let mut settings = HashMap::new();
    settings.insert(key.to_string(), value);
    state.config_service().update(&settings).await.unwrap();
}

fn kobo_sync_header(resp: &axum::http::Response<Body>) -> String {
    resp.headers()
        .get("x-kobo-sync")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

fn entitlement_id(entry: &serde_json::Value) -> String {
    entry
        .get("NewEntitlement")
        .or_else(|| entry.get("ChangedEntitlement"))
        .and_then(|bundle| bundle["BookEntitlement"]["Id"].as_str())
        .expect("entitlement id")
        .to_string()
}

fn all_removed(entries: &[serde_json::Value]) -> bool {
    entries.iter().all(|entry| {
        entry["ChangedEntitlement"]["BookEntitlement"]["IsRemoved"]
            .as_bool()
            .unwrap_or(false)
    })
}

#[tokio::test]
async fn first_sync_emits_new_then_second_sync_is_empty() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let user_token = register_and_login(&state, "alice").await;
    let (book_id, _file_id) = seed_epub_book(&state).await;
    let app = crate::build_router(state);

    let (pair, _) = pair_and_select_epub(&app, &user_token).await;

    // Enable selection.
    app.clone()
        .oneshot(req(
            Method::PUT,
            &format!("/api/books/{book_id}/kobo-sync"),
            Some(&user_token),
            Some(r#"{"enabled":true}"#),
        ))
        .await
        .unwrap();

    let url = format!("/kobo/{}/v1/library/sync", pair.token);
    let resp1 = app
        .clone()
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::OK);
    let entries: serde_json::Value = json_body(resp1).await;
    let arr = entries.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert!(arr[0].get("NewEntitlement").is_some());

    let resp2 = app
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();
    let entries2: serde_json::Value = json_body(resp2).await;
    assert_eq!(entries2.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn sync_pages_new_and_removed_entitlements() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    update_runtime_setting(
        &state,
        crate::kobo::KOBO_SYNC_PAGE_SIZE_KEY,
        serde_json::json!(3),
    )
    .await;
    let user_token = register_and_login(&state, "alice").await;
    let user = state
        .auth_service()
        .validate_session(&user_token)
        .await
        .unwrap();

    let mut book_ids = Vec::new();
    let page_limit = 3;
    for i in 0..=page_limit {
        let (book_id, file_id) = seed_unique_epub_book(&state, &format!("Paged Book {i:03}")).await;
        KoboSyncSelectionRepository::upsert(state.db_pool(), user.id, book_id, Some(file_id))
            .await
            .unwrap();
        book_ids.push(book_id);
    }

    let app = crate::build_router(state.clone());
    let (pair, _) = pair_and_select_epub(&app, &user_token).await;
    let url = format!("/kobo/{}/v1/library/sync", pair.token);

    let resp1 = app
        .clone()
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::OK);
    assert_eq!(kobo_sync_header(&resp1), "continue");
    let page1: Vec<serde_json::Value> = json_body(resp1).await;
    assert_eq!(page1.len(), page_limit);

    // A subsequent sync resumes from the ledger and sends only the remaining
    // item, not the first page again.
    let resp2 = app
        .clone()
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();
    assert_eq!(kobo_sync_header(&resp2), "done");
    let page2: Vec<serde_json::Value> = json_body(resp2).await;
    assert_eq!(page2.len(), 1);

    let mut seen = HashSet::new();
    for entry in page1.iter().chain(page2.iter()) {
        assert!(seen.insert(entitlement_id(entry)));
    }
    assert_eq!(seen.len(), book_ids.len());

    for book_id in &book_ids {
        KoboSyncSelectionRepository::delete(state.db_pool(), user.id, *book_id)
            .await
            .unwrap();
    }

    let remove1 = app
        .clone()
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();
    assert_eq!(kobo_sync_header(&remove1), "continue");
    let remove_page1: Vec<serde_json::Value> = json_body(remove1).await;
    assert_eq!(remove_page1.len(), page_limit);
    assert!(all_removed(&remove_page1));

    let remove2 = app
        .clone()
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();
    assert_eq!(kobo_sync_header(&remove2), "done");
    let remove_page2: Vec<serde_json::Value> = json_body(remove2).await;
    assert_eq!(remove_page2.len(), 1);
    assert!(all_removed(&remove_page2));

    let empty = app
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();
    assert_eq!(kobo_sync_header(&empty), "done");
    let empty_page: Vec<serde_json::Value> = json_body(empty).await;
    assert!(empty_page.is_empty());
}

#[tokio::test]
async fn sync_page_size_setting_controls_continuation_boundary() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    update_runtime_setting(
        &state,
        crate::kobo::KOBO_SYNC_PAGE_SIZE_KEY,
        serde_json::json!(2),
    )
    .await;
    let user_token = register_and_login(&state, "alice").await;
    let user = state
        .auth_service()
        .validate_session(&user_token)
        .await
        .unwrap();

    for i in 0..3 {
        let (book_id, file_id) =
            seed_unique_epub_book(&state, &format!("Configured Page Book {i}")).await;
        KoboSyncSelectionRepository::upsert(state.db_pool(), user.id, book_id, Some(file_id))
            .await
            .unwrap();
    }

    let app = crate::build_router(state);
    let (pair, _) = pair_and_select_epub(&app, &user_token).await;
    let url = format!("/kobo/{}/v1/library/sync", pair.token);

    let resp1 = app
        .clone()
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();
    assert_eq!(kobo_sync_header(&resp1), "continue");
    let page1: Vec<serde_json::Value> = json_body(resp1).await;
    assert_eq!(page1.len(), 2);

    let resp2 = app
        .clone()
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();
    assert_eq!(kobo_sync_header(&resp2), "done");
    let page2: Vec<serde_json::Value> = json_body(resp2).await;
    assert_eq!(page2.len(), 1);
}

#[tokio::test]
async fn disabled_kobo_sync_blocks_new_work_without_tombstoning_device_library() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let user_token = register_and_login(&state, "alice").await;
    let (book_id, _file_id) = seed_epub_book(&state).await;
    let app = crate::build_router(state.clone());
    let (pair, _) = pair_and_select_epub(&app, &user_token).await;

    update_runtime_setting(
        &state,
        crate::kobo::KOBO_ENABLED_KEY,
        serde_json::json!(false),
    )
    .await;

    let status_resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            "/api/kobo/status",
            Some(&user_token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(status_resp.status(), StatusCode::OK);
    let status: KoboStatusResponse = json_body(status_resp).await;
    assert!(!status.enabled);
    assert_eq!(status.active_device_count, 1);
    assert_eq!(status.device_count, 1);

    let pair_resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/kobo/devices",
            Some(&user_token),
            Some("{}"),
        ))
        .await
        .unwrap();
    assert_eq!(pair_resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    let select_resp = app
        .clone()
        .oneshot(req(
            Method::PUT,
            &format!("/api/books/{book_id}/kobo-sync"),
            Some(&user_token),
            Some(r#"{"enabled":true}"#),
        ))
        .await
        .unwrap();
    assert_eq!(select_resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    let sync_resp = app
        .oneshot(req(
            Method::GET,
            &format!("/kobo/{}/v1/library/sync", pair.token),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(sync_resp.status(), StatusCode::OK);
    assert_eq!(kobo_sync_header(&sync_resp), "done");
    let entries: Vec<serde_json::Value> = json_body(sync_resp).await;
    assert!(entries.is_empty());
}

#[tokio::test]
async fn unselect_emits_one_removed_entitlement() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let user_token = register_and_login(&state, "alice").await;
    let (book_id, _) = seed_epub_book(&state).await;
    let app = crate::build_router(state);

    let (pair, _) = pair_and_select_epub(&app, &user_token).await;
    app.clone()
        .oneshot(req(
            Method::PUT,
            &format!("/api/books/{book_id}/kobo-sync"),
            Some(&user_token),
            Some(r#"{"enabled":true}"#),
        ))
        .await
        .unwrap();

    // First sync: deliver.
    let url = format!("/kobo/{}/v1/library/sync", pair.token);
    let _ = app
        .clone()
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();

    // Disable.
    app.clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/books/{book_id}/kobo-sync"),
            Some(&user_token),
            None,
        ))
        .await
        .unwrap();

    // Next sync: removed entitlement once.
    let resp = app
        .clone()
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();
    let entries: serde_json::Value = json_body(resp).await;
    let arr = entries.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    let env = &arr[0];
    let bundle = env.get("ChangedEntitlement").expect("removed envelope");
    assert_eq!(bundle["BookEntitlement"]["IsRemoved"], true);

    // And subsequently empty.
    let resp = app
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();
    let entries: serde_json::Value = json_body(resp).await;
    assert_eq!(entries.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn revoked_token_returns_401() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let user_token = register_and_login(&state, "alice").await;
    let app = crate::build_router(state);
    let (pair, _) = pair_and_select_epub(&app, &user_token).await;

    app.clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/kobo/devices/{}", pair.id),
            Some(&user_token),
            None,
        ))
        .await
        .unwrap();

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/kobo/{}/v1/library/sync", pair.token),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unknown_kobo_store_probe_returns_benign_json() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let user_token = register_and_login(&state, "alice").await;
    let app = crate::build_router(state);
    let (pair, _) = pair_and_select_epub(&app, &user_token).await;

    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/kobo/{}/v1/products/featured/test-list", pair.token),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(resp).await;
    assert_eq!(body, serde_json::json!({}));

    let invalid = app
        .oneshot(req(
            Method::GET,
            "/kobo/not-a-valid-token/v1/products/featured/test-list",
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn initialization_preserves_known_onestore_resource_keys() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let user_token = register_and_login(&state, "alice").await;
    let app = crate::build_router(state);
    let (pair, _) = pair_and_select_epub(&app, &user_token).await;

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/kobo/{}/v1/initialization", pair.token),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(resp).await;
    let resources = body["Resources"].as_object().expect("Resources object");
    let keys: HashSet<&str> = resources.keys().map(String::as_str).collect();

    for key in [
        "dropbox_link_account_poll",
        "feedback",
        "googledrive_link_account_start",
        "instapaper_enabled",
        "instapaper_env_url",
        "instapaper_link_account_start",
        "kda_store_browser_redirect_url",
        "love_data",
        "subscription_publisher_price_page",
    ] {
        assert!(keys.contains(key), "missing OneStoreServices key {key}");
    }
}

#[tokio::test]
async fn download_returns_kepub_bytes_for_selected_file() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let user_token = register_and_login(&state, "alice").await;
    let (book_id, file_id) = seed_epub_book(&state).await;
    let app = crate::build_router(state);
    let (pair, _) = pair_and_select_epub(&app, &user_token).await;

    app.clone()
        .oneshot(req(
            Method::PUT,
            &format!("/api/books/{book_id}/kobo-sync"),
            Some(&user_token),
            Some(r#"{"enabled":true}"#),
        ))
        .await
        .unwrap();

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/kobo/{}/download/{}/{}", pair.token, book_id, file_id),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.contains("application/kepub+zip"),
        "got content-type {ct}"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(body.len() > 4 && &body[..2] == b"PK", "expected zip magic");
}

#[tokio::test]
async fn download_rejected_when_not_selected() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let user_token = register_and_login(&state, "alice").await;
    let (book_id, file_id) = seed_epub_book(&state).await;
    let app = crate::build_router(state);
    let (pair, _) = pair_and_select_epub(&app, &user_token).await;

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/kobo/{}/download/{}/{}", pair.token, book_id, file_id),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn sync_metadata_and_image_route_include_cover() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let user_token = register_and_login(&state, "alice").await;
    let (book_id, _) = seed_epub_book(&state).await;
    seed_cover(&state, book_id).await;
    let app = crate::build_router(state);
    let (pair, _) = pair_and_select_epub(&app, &user_token).await;

    app.clone()
        .oneshot(req(
            Method::PUT,
            &format!("/api/books/{book_id}/kobo-sync"),
            Some(&user_token),
            Some(r#"{"enabled":true}"#),
        ))
        .await
        .unwrap();

    let sync_resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/kobo/{}/v1/library/sync", pair.token),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(sync_resp.status(), StatusCode::OK);
    let entries: serde_json::Value = json_body(sync_resp).await;
    assert_eq!(
        entries[0]["NewEntitlement"]["BookMetadata"]["CoverImageId"],
        book_id.to_string()
    );

    let image_resp = app
        .oneshot(req(
            Method::GET,
            &format!(
                "/kobo/{}/v1/products/images/{}/300/400/90/false/image.jpg",
                pair.token, book_id
            ),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(image_resp.status(), StatusCode::OK);
    let ct = image_resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert_eq!(ct, "image/jpeg");
    let body = axum::body::to_bytes(image_resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], b"fake-jpeg");
}

#[tokio::test]
async fn changed_metadata_emits_changed_entitlement() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(&tmp, Some("https://example.test")).await;
    let user_token = register_and_login(&state, "alice").await;
    let (book_id, _) = seed_epub_book(&state).await;
    let app = crate::build_router(state);
    let (pair, _) = pair_and_select_epub(&app, &user_token).await;

    app.clone()
        .oneshot(req(
            Method::PUT,
            &format!("/api/books/{book_id}/kobo-sync"),
            Some(&user_token),
            Some(r#"{"enabled":true}"#),
        ))
        .await
        .unwrap();

    let url = format!("/kobo/{}/v1/library/sync", pair.token);
    // First sync delivers.
    let _ = app
        .clone()
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();

    // Confirm the book is reachable before we mutate it.
    let book_resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/api/books/{book_id}"),
            Some(&user_token),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(book_resp.status(), StatusCode::OK);

    // Change title via PUT /api/books/{id}.
    let upd = app
        .clone()
        .oneshot(req(
            Method::PUT,
            &format!("/api/books/{book_id}"),
            Some(&user_token),
            Some(r#"{"title":"Hello World - Updated"}"#),
        ))
        .await
        .unwrap();
    assert!(
        upd.status().is_success(),
        "title update should succeed; got {}",
        upd.status()
    );

    // Re-select to bump the selection_updated_at and recompute revision_hash.
    app.clone()
        .oneshot(req(
            Method::PUT,
            &format!("/api/books/{book_id}/kobo-sync"),
            Some(&user_token),
            Some(r#"{"enabled":true}"#),
        ))
        .await
        .unwrap();

    let resp = app
        .oneshot(req(Method::GET, &url, None, None))
        .await
        .unwrap();
    let entries: serde_json::Value = json_body(resp).await;
    let arr = entries.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert!(arr[0].get("ChangedEntitlement").is_some());
}
