use std::path::PathBuf;
use std::sync::Arc;

use archivis_auth::{AuthService, LocalAuthAdapter};
use archivis_db::DbPool;
use archivis_storage::local::LocalStorage;
use archivis_tasks::queue::TaskQueue;

/// API-specific configuration extracted from the application config.
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// Directory for application data (covers, cache, etc.).
    pub data_dir: PathBuf,
}

/// Shared application state passed to all API handlers.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    db_pool: DbPool,
    task_queue: Arc<TaskQueue>,
    auth_service: AuthService<LocalAuthAdapter>,
    storage: LocalStorage,
    config: ApiConfig,
}

impl AppState {
    pub fn new(
        db_pool: DbPool,
        task_queue: Arc<TaskQueue>,
        auth_service: AuthService<LocalAuthAdapter>,
        storage: LocalStorage,
        config: ApiConfig,
    ) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                db_pool,
                task_queue,
                auth_service,
                storage,
                config,
            }),
        }
    }

    pub fn db_pool(&self) -> &DbPool {
        &self.inner.db_pool
    }

    pub fn task_queue(&self) -> &TaskQueue {
        &self.inner.task_queue
    }

    pub fn auth_service(&self) -> &AuthService<LocalAuthAdapter> {
        &self.inner.auth_service
    }

    pub fn storage(&self) -> &LocalStorage {
        &self.inner.storage
    }

    pub fn config(&self) -> &ApiConfig {
        &self.inner.config
    }
}
