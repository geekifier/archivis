use std::sync::Arc;

use archivis_auth::{AuthService, LocalAuthAdapter};
use archivis_db::DbPool;
use archivis_tasks::queue::TaskQueue;

/// Shared application state passed to all API handlers.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    db_pool: DbPool,
    task_queue: Arc<TaskQueue>,
    auth_service: AuthService<LocalAuthAdapter>,
}

impl AppState {
    pub fn new(
        db_pool: DbPool,
        task_queue: Arc<TaskQueue>,
        auth_service: AuthService<LocalAuthAdapter>,
    ) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                db_pool,
                task_queue,
                auth_service,
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
}
