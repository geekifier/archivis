pub mod auth;
pub mod authors;
pub mod books;
pub mod errors;
pub mod import;
pub mod series;
pub mod state;
pub mod tags;
pub mod tasks;

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
            super::books::handlers::download_file,
            super::books::handlers::set_book_authors,
            super::books::handlers::set_book_tags,
            // Authors
            super::authors::handlers::list_authors,
            super::authors::handlers::create_author,
            super::authors::handlers::get_author,
            super::authors::handlers::update_author,
            super::authors::handlers::delete_author,
            super::authors::handlers::list_author_books,
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
            // Authors
            super::authors::types::CreateAuthorRequest,
            super::authors::types::UpdateAuthorRequest,
            super::authors::types::AuthorResponse,
            super::authors::types::PaginatedAuthors,
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
        )),
        tags(
            (name = "auth", description = "Authentication and user management"),
            (name = "books", description = "Book CRUD operations"),
            (name = "authors", description = "Author management"),
            (name = "series", description = "Series management"),
            (name = "tags", description = "Tag management"),
            (name = "import", description = "File and directory import"),
            (name = "tasks", description = "Background task management"),
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
        .nest("/series", series::router())
        .nest("/tags", tags::router())
        .nest("/import", import::router());

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

        AppState::new(
            db_pool,
            Arc::new(task_queue),
            auth_service,
            storage,
            ApiConfig {
                data_dir: dir.to_path_buf(),
                frontend_dir,
            },
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
}
