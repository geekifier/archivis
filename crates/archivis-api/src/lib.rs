pub mod auth;
pub mod books;
pub mod errors;
pub mod state;
pub mod tasks;

use axum::Router;
use tower_http::cors::CorsLayer;
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
            super::auth::handlers::auth_status,
            super::auth::handlers::auth_setup,
            super::auth::handlers::auth_login,
            super::auth::handlers::auth_logout,
            super::auth::handlers::auth_me,
            super::books::handlers::list_books,
            super::books::handlers::get_book,
            super::books::handlers::update_book,
            super::books::handlers::delete_book,
            super::books::handlers::get_cover,
            super::books::handlers::download_file,
            super::books::handlers::set_book_authors,
            super::books::handlers::set_book_tags,
        ),
        components(schemas(
            super::auth::types::SetupRequest,
            super::auth::types::LoginRequest,
            super::auth::types::AuthStatusResponse,
            super::auth::types::LoginResponse,
            super::auth::types::UserResponse,
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
        )),
        tags(
            (name = "tasks", description = "Background task management"),
            (name = "auth", description = "Authentication and user management"),
            (name = "books", description = "Book CRUD operations"),
            (name = "authors", description = "Author management"),
            (name = "series", description = "Series management"),
            (name = "tags", description = "Tag management"),
            (name = "import", description = "File and directory import"),
        )
    )]
    pub struct ApiDoc;
}

/// Build the full API router with all route groups and middleware.
pub fn build_router(state: AppState) -> Router {
    let api_routes = Router::new()
        .nest("/tasks", tasks::router())
        .nest("/auth", auth::router())
        .nest("/books", books::router())
        .nest("/authors", stub_router())
        .nest("/series", stub_router())
        .nest("/tags", stub_router())
        .nest("/import", stub_router());

    Router::new()
        .merge(SwaggerUi::new("/api/docs").url("/api/openapi.json", openapi::ApiDoc::openapi()))
        .nest("/api", api_routes)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Placeholder router for route groups that will be implemented in later tasks.
fn stub_router() -> Router<AppState> {
    Router::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ApiConfig;
    use std::sync::Arc;

    /// Smoke test: constructing the router must not panic (e.g. overlapping routes).
    #[tokio::test]
    async fn build_router_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let storage_dir = tmp.path().join("books");

        let db_pool = archivis_db::create_pool(&db_path).await.unwrap();
        archivis_db::run_migrations(&db_pool).await.unwrap();

        let storage = archivis_storage::local::LocalStorage::new(&storage_dir)
            .await
            .unwrap();
        let auth_adapter = archivis_auth::LocalAuthAdapter::new(db_pool.clone());
        let auth_service = archivis_auth::AuthService::new(db_pool.clone(), auth_adapter);
        let (task_queue, _rx) = archivis_tasks::queue::TaskQueue::new(db_pool.clone());

        let state = AppState::new(
            db_pool,
            Arc::new(task_queue),
            auth_service,
            storage,
            ApiConfig {
                data_dir: tmp.path().to_path_buf(),
            },
        );

        // This is the line that would panic on overlapping routes.
        let _router = build_router(state);
    }
}
