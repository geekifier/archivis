pub mod auth;
pub mod authors;
pub mod books;
pub mod duplicates;
pub mod errors;
pub mod filesystem;
pub mod identify;
pub mod import;
pub mod isbn_scan;
pub mod publishers;
pub mod reader;
pub mod series;
pub mod settings;
pub mod state;
pub mod stats;
pub mod tags;
pub mod tasks;
pub mod watcher;

use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::state::AppState;

// The `OpenApi` derive macro generates code that triggers `needless_for_each`.
#[allow(clippy::needless_for_each)]
mod openapi {
    use utoipa::OpenApi;

    #[derive(OpenApi)]
    #[openapi(
        info(
            title = "Archivis API",
            description = "REST API for the Archivis e-book collection manager",
            version = "0.1.0",
            license(name = "AGPL-3.0-or-later"),
        ),
        paths(
            // Auth
            super::auth::handlers::auth_status,
            super::auth::handlers::auth_setup,
            super::auth::handlers::auth_login,
            super::auth::handlers::auth_logout,
            super::auth::handlers::auth_me,
            // Books
            super::books::handlers::list_books,
            super::books::handlers::get_book,
            super::books::handlers::update_book,
            super::books::handlers::delete_book,
            super::books::handlers::get_cover,
            super::books::handlers::upload_cover,
            super::books::handlers::download_file,
            super::books::handlers::set_book_authors,
            super::books::handlers::set_book_series,
            super::books::handlers::set_book_tags,
            super::books::handlers::add_identifier,
            super::books::handlers::update_identifier,
            super::books::handlers::delete_identifier,
            super::books::handlers::batch_update_books,
            super::books::handlers::batch_set_tags,
            super::books::handlers::serve_file_content,
            // Authors
            super::authors::handlers::list_authors,
            super::authors::handlers::create_author,
            super::authors::handlers::get_author,
            super::authors::handlers::update_author,
            super::authors::handlers::delete_author,
            super::authors::handlers::list_author_books,
            // Publishers
            super::publishers::handlers::list_publishers,
            super::publishers::handlers::create_publisher,
            super::publishers::handlers::get_publisher,
            super::publishers::handlers::update_publisher,
            super::publishers::handlers::delete_publisher,
            super::publishers::handlers::list_publisher_books,
            // Series
            super::series::handlers::list_series,
            super::series::handlers::create_series,
            super::series::handlers::get_series,
            super::series::handlers::update_series,
            super::series::handlers::delete_series,
            super::series::handlers::list_series_books,
            // Tags
            super::tags::handlers::list_tags,
            super::tags::handlers::create_tag,
            super::tags::handlers::get_tag,
            super::tags::handlers::update_tag,
            super::tags::handlers::delete_tag,
            super::tags::handlers::list_tag_books,
            // Import
            super::import::handlers::upload_files,
            super::import::handlers::scan_directory,
            super::import::handlers::start_import,
            // Tasks
            super::tasks::handlers::list_tasks,
            super::tasks::handlers::get_task,
            super::tasks::sse::task_progress_sse,
            super::tasks::sse::active_tasks_sse,
            // Identify
            super::identify::handlers::identify_book,
            super::identify::handlers::list_candidates,
            super::identify::handlers::apply_candidate,
            super::identify::handlers::reject_candidate,
            super::identify::handlers::undo_candidate,
            super::identify::handlers::batch_identify,
            super::identify::handlers::identify_all,
            // Reader
            super::reader::handlers::get_progress,
            super::reader::handlers::update_progress,
            super::reader::handlers::delete_progress,
            super::reader::handlers::continue_reading,
            super::reader::handlers::list_bookmarks,
            super::reader::handlers::create_bookmark,
            super::reader::handlers::update_bookmark,
            super::reader::handlers::delete_bookmark,
            // ISBN Scan
            super::isbn_scan::handlers::scan_book_isbn,
            super::isbn_scan::handlers::batch_scan_isbn,
            // Duplicates
            super::duplicates::handlers::list_duplicates,
            super::duplicates::handlers::count_duplicates,
            super::duplicates::handlers::get_duplicate,
            super::duplicates::handlers::merge_duplicate,
            super::duplicates::handlers::dismiss_duplicate,
            super::duplicates::handlers::flag_duplicate,
            // Filesystem
            super::filesystem::handlers::browse_directory,
            // Settings
            super::settings::handlers::get_settings,
            super::settings::handlers::update_settings,
            // Stats
            super::stats::handlers::get_stats,
            // Watched Directories
            super::watcher::handlers::list_watched,
            super::watcher::handlers::add_watched,
            super::watcher::handlers::get_watched,
            super::watcher::handlers::update_watched,
            super::watcher::handlers::remove_watched,
            super::watcher::handlers::trigger_scan,
            super::watcher::handlers::detect_fs,
        ),
        components(schemas(
            // Auth
            super::auth::types::SetupRequest,
            super::auth::types::LoginRequest,
            super::auth::types::AuthStatusResponse,
            super::auth::types::LoginResponse,
            super::auth::types::UserResponse,
            // Books
            super::books::types::UpdateBookRequest,
            super::books::types::SetBookAuthorsRequest,
            super::books::types::BookAuthorLink,
            super::books::types::SetBookSeriesRequest,
            super::books::types::BookSeriesLink,
            super::books::types::SetBookTagsRequest,
            super::books::types::BookTagLink,
            super::books::types::BookSummary,
            super::books::types::BookDetail,
            super::books::types::AuthorEntry,
            super::books::types::SeriesEntry,
            super::books::types::TagEntry,
            super::books::types::FileEntry,
            super::books::types::IdentifierEntry,
            super::books::types::PaginatedBooks,
            super::books::types::AddIdentifierRequest,
            super::books::types::UpdateIdentifierRequest,
            super::books::types::BatchUpdateBooksRequest,
            super::books::types::BatchBookFields,
            super::books::types::BatchSetTagsRequest,
            super::books::types::BatchTagMode,
            super::books::types::BatchUpdateResponse,
            super::books::types::BatchTagsResponse,
            super::books::types::BatchUpdateError,
            // Authors
            super::authors::types::CreateAuthorRequest,
            super::authors::types::UpdateAuthorRequest,
            super::authors::types::AuthorResponse,
            super::authors::types::PaginatedAuthors,
            // Publishers
            super::publishers::types::CreatePublisherRequest,
            super::publishers::types::UpdatePublisherRequest,
            super::publishers::types::PublisherResponse,
            super::publishers::types::PaginatedPublishers,
            // Series
            super::series::types::CreateSeriesRequest,
            super::series::types::UpdateSeriesRequest,
            super::series::types::SeriesResponse,
            super::series::types::PaginatedSeries,
            // Tags
            super::tags::types::CreateTagRequest,
            super::tags::types::UpdateTagRequest,
            super::tags::types::TagResponse,
            super::tags::types::PaginatedTags,
            // Import
            super::import::types::ScanDirectoryRequest,
            super::import::types::StartImportRequest,
            super::import::types::ScanManifestResponse,
            super::import::types::FormatSummary,
            super::import::types::TaskCreatedResponse,
            super::import::types::UploadResponse,
            // Tasks
            super::tasks::types::TaskResponse,
            // Identify
            super::identify::types::CandidateResponse,
            super::identify::types::SeriesInfo,
            super::identify::types::IdentifyResponse,
            super::identify::types::ApplyCandidateBody,
            super::identify::types::BatchIdentifyRequest,
            super::identify::types::IdentifyAllRequest,
            super::identify::types::IdentifyAllResponse,
            // Reader
            super::reader::types::ReadingProgressResponse,
            super::reader::types::UpdateProgressRequest,
            super::reader::types::ContinueReadingItem,
            super::reader::types::CreateBookmarkRequest,
            super::reader::types::BookmarkResponse,
            super::reader::handlers::UpdateBookmarkRequest,
            // ISBN Scan
            super::isbn_scan::types::IsbnScanResponse,
            super::isbn_scan::types::BatchIsbnScanRequest,
            super::isbn_scan::types::BatchIsbnScanResponse,
            // Duplicates
            super::duplicates::types::DuplicateLinkResponse,
            super::duplicates::types::PaginatedDuplicates,
            super::duplicates::types::MergeRequest,
            super::duplicates::types::FlagDuplicateRequest,
            super::duplicates::types::DuplicateCountResponse,
            // Filesystem
            super::filesystem::types::FsEntry,
            super::filesystem::types::BrowseResponse,
            // Settings
            super::settings::types::SettingsResponse,
            super::settings::types::UpdateSettingsRequest,
            super::settings::types::UpdateSettingsResponse,
            super::settings::service::SettingEntry,
            super::settings::service::ConfigSource,
            super::settings::service::ConfigOverride,
            super::settings::registry::SettingType,
            // Watched Directories
            super::watcher::types::AddWatchedDirectoryRequest,
            super::watcher::types::UpdateWatchedDirectoryRequest,
            super::watcher::types::WatchedDirectoryResponse,
            super::watcher::types::FsDetectionResponse,
            super::watcher::types::DetectFsRequest,
            super::watcher::types::ScanTriggeredResponse,
            // Stats
            super::stats::types::StatsResponse,
            super::stats::types::LibraryStats,
            super::stats::types::FormatStat,
            super::stats::types::StatusCount,
            super::stats::types::UsageStats,
            super::stats::types::TaskTypeCount,
            super::stats::types::DbStats,
            super::stats::types::DbFileStats,
            super::stats::types::DbPageStats,
            super::stats::types::DbObjectStatResponse,
        )),
        tags(
            (name = "auth", description = "Authentication and user management"),
            (name = "books", description = "Book CRUD operations"),
            (name = "authors", description = "Author management"),
            (name = "publishers", description = "Publisher management"),
            (name = "series", description = "Series management"),
            (name = "tags", description = "Tag management"),
            (name = "reader", description = "Reading progress and bookmarks"),
            (name = "import", description = "File and directory import"),
            (name = "identify", description = "Book metadata identification"),
            (name = "isbn-scan", description = "ISBN content scanning"),
            (name = "tasks", description = "Background task management"),
            (name = "duplicates", description = "Duplicate book management and merging"),
            (name = "filesystem", description = "Server filesystem browsing"),
            (name = "settings", description = "Instance settings management"),
            (name = "stats", description = "Library and usage statistics"),
            (name = "watched-directories", description = "Watched directory management"),
        )
    )]
    pub struct ApiDoc;
}

/// Build the full API router with all route groups and middleware.
///
/// When `ApiConfig::frontend_dir` is set and points to an existing directory
/// containing a built `SvelteKit` frontend, the router serves those static files
/// and falls back to `index.html` for SPA client-side routing.
pub fn build_router(state: AppState) -> Router {
    let api_routes = Router::new()
        .nest("/tasks", tasks::router())
        .nest("/auth", auth::router())
        .nest("/books", books::router())
        .nest("/authors", authors::router())
        .nest("/publishers", publishers::router())
        .nest("/series", series::router())
        .nest("/tags", tags::router())
        .nest("/reader", reader::router())
        .nest("/import", import::router())
        .nest("/identify", identify::router())
        .nest("/isbn-scan", isbn_scan::router())
        .nest("/duplicates", duplicates::router())
        .nest("/filesystem", filesystem::router())
        .nest("/settings", settings::router())
        .nest("/stats", stats::router())
        .nest("/watched-directories", watcher::router());

    let mut router = Router::new()
        .merge(SwaggerUi::new("/api/docs").url("/api/openapi.json", openapi::ApiDoc::openapi()))
        .nest("/api", api_routes);

    // Serve the built frontend from a configured directory.
    // Any request that does not match `/api/*` or Swagger UI is served from
    // the frontend dist directory. Paths that don't match a static file fall
    // back to `index.html` so that the SPA's client-side router can handle them.
    if let Some(ref dir) = state.config().frontend_dir {
        if dir.is_dir() {
            let index = dir.join("index.html");
            let spa_fallback = ServeDir::new(dir).fallback(ServeFile::new(index));
            router = router.fallback_service(spa_fallback);
            tracing::info!(path = %dir.display(), "Serving frontend assets");
        } else {
            tracing::warn!(
                path = %dir.display(),
                "Frontend directory does not exist — static file serving disabled"
            );
        }
    }

    router
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ApiConfig;
    use axum::body::Body;
    use http::{Request, StatusCode};
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Build a test `AppState` rooted in `dir`. The caller is responsible for
    /// keeping the `tempfile::TempDir` alive for the duration of the test.
    async fn test_state(
        dir: &std::path::Path,
        frontend_dir: Option<std::path::PathBuf>,
    ) -> AppState {
        let db_path = dir.join("test.db");
        let storage_dir = dir.join("books");

        let db_pool = archivis_db::create_pool(&db_path).await.unwrap();
        archivis_db::run_migrations(&db_pool).await.unwrap();

        let storage = archivis_storage::local::LocalStorage::new(&storage_dir)
            .await
            .unwrap();
        let auth_adapter = archivis_auth::LocalAuthAdapter::new(db_pool.clone());
        let auth_service = archivis_auth::AuthService::new(db_pool.clone(), auth_adapter);
        let (task_queue, _rx) = archivis_tasks::queue::TaskQueue::new(db_pool.clone());

        let provider_registry = Arc::new(archivis_metadata::ProviderRegistry::new());

        let resolver = Arc::new(archivis_metadata::MetadataResolver::new(
            Arc::clone(&provider_registry),
            0.85,
        ));
        let identify_service = Arc::new(archivis_tasks::identify::IdentificationService::new(
            db_pool.clone(),
            resolver,
            storage.clone(),
            dir.to_path_buf(),
        ));

        let merge_service = Arc::new(archivis_tasks::merge::MergeService::new(
            db_pool.clone(),
            storage.clone(),
            dir.to_path_buf(),
        ));

        let config_service = Arc::new(crate::settings::service::ConfigService::new(
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            db_pool.clone(),
        ));

        AppState::new(
            db_pool,
            Arc::new(task_queue),
            auth_service,
            storage,
            provider_registry,
            identify_service,
            merge_service,
            ApiConfig {
                data_dir: dir.to_path_buf(),
                frontend_dir,
            },
            config_service,
            None,
        )
    }

    /// Smoke test: constructing the router must not panic (e.g. overlapping routes).
    #[tokio::test]
    async fn build_router_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path(), None).await;
        let _router = build_router(state);
    }

    /// When `frontend_dir` is set, static files in that directory are served.
    #[tokio::test]
    async fn serves_static_files_from_frontend_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dist = tmp.path().join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("index.html"), "<html>app</html>").unwrap();
        std::fs::write(dist.join("style.css"), "body{}").unwrap();

        let state = test_state(tmp.path(), Some(dist)).await;
        let router = build_router(state);

        // Exact file should be served with correct content type.
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/style.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.contains("css"), "expected CSS content-type, got {ct}");
    }

    /// Unknown paths should fall back to `index.html` for SPA routing.
    #[tokio::test]
    async fn spa_fallback_returns_index_html() {
        let tmp = tempfile::tempdir().unwrap();
        let dist = tmp.path().join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("index.html"), "<html>spa</html>").unwrap();

        let state = test_state(tmp.path(), Some(dist)).await;
        let router = build_router(state);

        let resp = router
            .oneshot(
                Request::builder()
                    .uri("/books/some-id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body, "<html>spa</html>");
    }

    /// API routes must not be shadowed by the static file fallback.
    #[tokio::test]
    async fn api_routes_take_priority_over_frontend() {
        let tmp = tempfile::tempdir().unwrap();
        let dist = tmp.path().join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("index.html"), "<html>spa</html>").unwrap();

        let state = test_state(tmp.path(), Some(dist)).await;
        let router = build_router(state);

        // The /api/auth/status endpoint should still work.
        let resp = router
            .oneshot(
                Request::builder()
                    .uri("/api/auth/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Should be the API response, not the SPA index.html
        assert!(json.get("setup_required").is_some());
    }

    /// When `frontend_dir` is `None`, no static file serving is configured.
    #[tokio::test]
    async fn no_frontend_dir_returns_404_for_root() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path(), None).await;
        let router = build_router(state);

        let resp = router
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// When `frontend_dir` points to a non-existent directory, behave like `None`.
    #[tokio::test]
    async fn nonexistent_frontend_dir_returns_404() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path(), Some(tmp.path().join("does-not-exist"))).await;
        let router = build_router(state);

        let resp = router
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn stats_endpoint_hides_db_stats_for_non_admin() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path(), None).await;

        state
            .auth_service()
            .register("admin", "password123", None)
            .await
            .unwrap();
        state
            .auth_service()
            .register("reader", "password123", None)
            .await
            .unwrap();

        let (token, _session) = state
            .auth_service()
            .login("reader", "password123")
            .await
            .unwrap();

        let router = build_router(state);
        let resp = router
            .oneshot(
                Request::builder()
                    .uri("/api/stats")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("library").is_some());
        assert!(json.get("usage").is_some());
        assert!(json["db"].is_null());
    }

    #[tokio::test]
    async fn stats_endpoint_includes_db_stats_for_admin() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path(), None).await;

        state
            .auth_service()
            .register("admin", "password123", None)
            .await
            .unwrap();

        let (token, _session) = state
            .auth_service()
            .login("admin", "password123")
            .await
            .unwrap();

        let router = build_router(state);
        let resp = router
            .oneshot(
                Request::builder()
                    .uri("/api/stats")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("library").is_some());
        assert!(json.get("usage").is_some());
        assert!(json["db"].is_object());
    }
}
